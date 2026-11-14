//! Per-machine connection manager: one tokio task per machine, state machine with
//! reconnect backoff (1, 2, 4 .. 30 s). Machines never block each other.
use crate::api::{ApiClient, Workspace};
use crate::machines::Machine;
use crate::surface::{self, Decoder, Frame, OutCell, Surface};
use crate::transport::{for_machine, Channel, Transport};
use crate::{Error, Result};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::Instant;

pub const BACKOFF_MAX: u64 = 30;
pub const HANDSHAKE: Duration = Duration::from_secs(5);
pub const HEALTH_INTERVAL: Duration = Duration::from_secs(5);
pub const HEALTH_DEADLINE: Duration = Duration::from_secs(10);
pub const CELL_PX: (u32, u32) = (9, 18);

/// Backoff delay in seconds for the n-th consecutive failure (n starts at 0): 1, 2, 4 .. 30.
pub fn backoff_secs(failures: u32) -> u64 {
    (1u64 << failures.min(5)).min(BACKOFF_MAX)
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Connected,
    Connecting,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct CursorOut {
    pub x: usize,
    pub y: usize,
    pub visible: bool,
    pub shape: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaneSurface {
    pub pane_id: String,
    pub x: usize,
    pub y: usize,
    pub cols: usize,
    pub rows: usize,
    pub focused: bool,
    pub cursor: CursorOut,
    pub rows_data: Vec<Vec<OutCell>>,
    pub dirty_rows: Vec<usize>,
}

/// Events emitted to the host (the Tauri glue maps these 1:1 onto app events).
#[derive(Debug, Clone)]
pub enum Event {
    Status { machine_id: String, state: State, reason: Option<String>, retry_in_secs: Option<u64> },
    Snapshot { machine_id: String, workspaces: Vec<Workspace> },
    Surface { machine_id: String, tab_id: String, panes: Vec<PaneSurface> },
}

#[derive(Debug)]
pub enum Command {
    Reconnect,
    OpenSpace(String),
    SelectTab(String),
    FocusPane(String),
    Key { pane_id: String, text: Option<String>, code: String, ctrl: bool, alt: bool, shift: bool },
    Paste { pane_id: String, text: String },
    Resize { cols: u32, rows: u32 },
    Scroll { pane_id: String, lines: i32 },
    Stop,
}

/// Handle to a running machine task. Sending never blocks.
#[derive(Clone)]
pub struct MachineHandle {
    pub machine_id: String,
    tx: UnboundedSender<Command>,
}

impl MachineHandle {
    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }
}

/// Starts the task for one machine. Must be called inside a tokio runtime.
pub fn spawn_machine(m: Machine, events: UnboundedSender<Event>) -> MachineHandle {
    let (tx, rx) = unbounded_channel();
    let id = m.id.clone();
    tokio::spawn(run_machine(m, events, rx));
    MachineHandle { machine_id: id, tx }
}

fn status(events: &UnboundedSender<Event>, id: &str, state: State, reason: Option<String>, retry: Option<u64>) {
    let _ = events.send(Event::Status { machine_id: id.into(), state, reason, retry_in_secs: retry });
}

async fn run_machine(m: Machine, events: UnboundedSender<Event>, mut rx: UnboundedReceiver<Command>) {
    let transport: Arc<dyn Transport> = match for_machine(&m) {
        Ok(t) => t,
        Err(e) => {
            status(&events, &m.id, State::Failed, Some(e.to_string()), None);
            return;
        }
    };
    let mut failures = 0u32;
    let mut cfg = ViewState { cols: 120, rows: 40 };
    loop {
        status(&events, &m.id, State::Connecting, None, None);
        let mut connected = false;
        let res = session(&*transport, &m.id, &events, &mut rx, &mut cfg, &mut connected).await;
        match res {
            Ok(Exit::Stop) => return,
            Ok(Exit::Reconnect) => {
                failures = 0;
                continue;
            }
            Err(e) => {
                if connected {
                    failures = 0;
                }
                let mut reason = e.to_string();
                if let Some(d) = transport.diagnostics().filter(|d| !d.trim().is_empty()) {
                    reason = format!("{reason} ({})", d.trim());
                }
                let wait = backoff_secs(failures);
                failures = failures.saturating_add(1);
                status(&events, &m.id, State::Failed, Some(reason), Some(wait));
                let sleep = tokio::time::sleep(Duration::from_secs(wait));
                tokio::pin!(sleep);
                loop {
                    tokio::select! {
                        _ = &mut sleep => break,
                        c = rx.recv() => match c {
                            None | Some(Command::Stop) => return,
                            Some(Command::Reconnect) => { failures = 0; break }
                            Some(Command::Resize { cols, rows }) => { cfg.cols = cols; cfg.rows = rows; }
                            Some(_) => {}
                        }
                    }
                }
            }
        }
    }
}

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

enum Exit {
    Stop,
    Reconnect,
}

struct ViewState {
    cols: u32,
    rows: u32,
}

async fn session(
    t: &dyn Transport,
    id: &str,
    events: &UnboundedSender<Event>,
    rx: &mut UnboundedReceiver<Command>,
    cfg: &mut ViewState,
    connected: &mut bool,
) -> Result<Exit> {
    // JSON API: subscribe first, then snapshot.
    let api_stream = tokio::time::timeout(HANDSHAKE, t.connect(Channel::Api))
        .await
        .map_err(|_| Error::Timeout("api connect"))??;
    let mut api = ApiClient::new(api_stream);
    api.subscribe().await?;
    let snap = api.snapshot().await?;
    let mut focused_tab = snap.focused_tab_id.clone().unwrap_or_default();
    let _ = events.send(Event::Snapshot { machine_id: id.into(), workspaces: snap.workspaces.clone() });
    let mut workspaces = snap.workspaces;
    // Dedicated reader task owns the API connection; the select! below only awaits the channel.
    let (api_tx, mut api_rx) = tokio::sync::mpsc::channel::<Result<crate::api::Snapshot>>(8);
    let api_task = AbortOnDrop(tokio::spawn(async move {
        loop {
            if let Err(e) = api.next_event().await {
                let _ = api_tx.send(Err(e)).await;
                return;
            }
            let r = api.snapshot().await;
            let failed = r.is_err();
            if api_tx.send(r).await.is_err() || failed {
                return;
            }
        }
    }));

    // Binary client endpoint.
    let client = tokio::time::timeout(HANDSHAKE, t.connect(Channel::Client))
        .await
        .map_err(|_| Error::Timeout("client connect"))??;
    let (crd_, mut cwr) = tokio::io::split(client);
    let (cw, ch) = CELL_PX;
    cwr.write_all(&surface::frame(&surface::hello(cw, ch, cfg.cols, cfg.rows))).await?;
    cwr.write_all(&surface::frame(&surface::focus(true))).await?;

    let mut crd = crd_;
    let mut dec = Decoder::new();
    let (fr_tx, mut fr_rx) = tokio::sync::mpsc::channel::<Result<Vec<Vec<u8>>>>(64);
    let _frame_task = AbortOnDrop(tokio::spawn(async move {
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 65536];
        loop {
            let r: Result<Vec<Vec<u8>>> = match crd.read(&mut chunk).await {
                Ok(0) => Err(Error::protocol("client endpoint closed")),
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    surface::drain_frames(&mut buf)
                }
                Err(e) => Err(e.into()),
            };
            let stop = r.is_err();
            if fr_tx.send(r).await.is_err() || stop {
                return;
            }
        }
    }));
    let _keep = &api_task;
    let mut welcomed = false;
    let started = Instant::now();
    let mut last_rx = Instant::now();
    let mut ping = tokio::time::interval(HEALTH_INTERVAL);
    let mut ping_n = 0u64;

    loop {
        tokio::select! {
            frames = fr_rx.recv() => {
                let frames = frames.ok_or_else(|| Error::protocol("reader stopped"))??;
                last_rx = Instant::now();
                for f in frames {
                    match dec.apply(&f) {
                        Ok(Frame::Full) => {
                            emit_surface(events, id, &focused_tab, dec.surface.as_ref().unwrap(), None);
                        }
                        Ok(Frame::Patch { dirty_rows }) => {
                            emit_surface(events, id, &focused_tab, dec.surface.as_ref().unwrap(), Some(&dirty_rows));
                        }
                        Ok(Frame::Control { kind, .. }) => {
                            if kind == "endpoint.welcome.v1" && !welcomed {
                                welcomed = true;
                                *connected = true;
                                status(events, id, State::Connected, None, None);
                            }
                        }
                        Ok(_) => {}
                        // A frame we cannot apply desynchronises the grid: reconnect for a fresh full surface.
                        Err(e) => return Err(e),
                    }
                }
            }
            snap = api_rx.recv() => {
                // Any change event made the reader task re-fetch the snapshot.
                let snap = snap.ok_or_else(|| Error::protocol("api reader stopped"))??;
                if let Some(ft) = snap.focused_tab_id { focused_tab = ft; }
                workspaces = snap.workspaces;
                let _ = events.send(Event::Snapshot { machine_id: id.into(), workspaces: workspaces.clone() });
            }
            _ = ping.tick() => {
                if !welcomed && started.elapsed() > HANDSHAKE { return Err(Error::Timeout("endpoint handshake")); }
                if last_rx.elapsed() > HEALTH_DEADLINE { return Err(Error::Timeout("health reply")); }
                ping_n += 1;
                write(&mut cwr, &surface::ping(&ping_n.to_string())).await?;
            }
            c = rx.recv() => {
                let Some(c) = c else { return Ok(Exit::Stop) };
                match c {
                    Command::Stop => return Ok(Exit::Stop),
                    Command::Reconnect => return Ok(Exit::Reconnect),
                    Command::Resize { cols, rows } => {
                        cfg.cols = cols; cfg.rows = rows;
                        write(&mut cwr, &surface::resize(cw, ch, cols, rows)).await?;
                    }
                    Command::OpenSpace(ws_id) => {
                        if let Some(w) = workspaces.iter().find(|w| w.id == ws_id) {
                            if let Some(t) = w.tabs.iter().find(|t| t.focused).or(w.tabs.first()) {
                                focused_tab = t.id.clone();
                            }
                        }
                        api_via_endpoint(&mut cwr, &dec, "workspace.focus", serde_json::json!({"workspace_id": ws_id})).await?;
                        resend_surface(events, id, &focused_tab, &dec);
                    }
                    Command::SelectTab(tab_id) => {
                        focused_tab = tab_id.clone();
                        api_via_endpoint(&mut cwr, &dec, "tab.focus", serde_json::json!({"tab_id": tab_id})).await?;
                        resend_surface(events, id, &focused_tab, &dec);
                    }
                    Command::FocusPane(pane_id) => {
                        api_via_endpoint(&mut cwr, &dec, "pane.focus", serde_json::json!({"pane_id": pane_id})).await?;
                    }
                    Command::Key { pane_id, text, code, ctrl, alt, shift } => {
                        let plain = !ctrl && !alt;
                        let msg = match text.as_deref() {
                            Some(t) if plain && !t.is_empty() && t.chars().count() >= 1 && code.chars().count() != 0
                                && surface::key_code(&code, shift).is_none() => Some(surface::input_text(&pane_id, t)),
                            _ => surface::input_key(&pane_id, &code, ctrl, alt, shift),
                        };
                        if let Some(m) = msg { write(&mut cwr, &m).await?; }
                    }
                    Command::Paste { pane_id, text } => write(&mut cwr, &surface::input_paste(&pane_id, &text)).await?,
                    Command::Scroll { pane_id, lines } => {
                        if lines != 0 {
                            let (col, row) = dec.surface.as_ref().and_then(|s| s.panes.iter().find(|p| p.id == pane_id))
                                .map(|p| (p.inner.x + p.inner.w / 2, p.inner.y + p.inner.h / 2)).unwrap_or((0, 0));
                            let m = surface::input_wheel(&pane_id, lines > 0, col as u32, row as u32, lines.unsigned_abs());
                            write(&mut cwr, &m).await?;
                        }
                    }
                }
            }
        }
    }
}

async fn write<W: AsyncWriteExt + Unpin>(w: &mut W, payload: &[u8]) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        w.write_all(&surface::frame(payload)).await?;
        w.flush().await
    })
    .await
    .map_err(|_| Error::Timeout("write"))??;
    Ok(())
}

async fn api_via_endpoint<W: AsyncWriteExt + Unpin>(
    w: &mut W,
    dec: &Decoder,
    method: &str,
    params: serde_json::Value,
) -> Result<()> {
    let Some(sf) = &dec.surface else { return Ok(()) };
    let rid = uuid::Uuid::new_v4().to_string();
    write(w, &surface::api_request(&sf.boot_id, &rid, method, params)).await
}

fn resend_surface(events: &UnboundedSender<Event>, id: &str, tab: &str, dec: &Decoder) {
    if let Some(sf) = &dec.surface {
        emit_surface(events, id, tab, sf, None);
    }
}

/// Splits the whole-screen grid into per-pane full grids. `dirty` limits the dirty-row list
/// (surface rows); `None` means everything is dirty.
pub fn pane_surfaces(sf: &Surface, dirty: Option<&[usize]>) -> Vec<PaneSurface> {
    sf.panes
        .iter()
        .filter(|p| p.inner.w > 0 && p.inner.h > 0 && p.inner.x + p.inner.w <= sf.width && p.inner.y + p.inner.h <= sf.height)
        .map(|p| {
            let r = p.inner;
            let rows_data: Vec<Vec<OutCell>> = (r.y..r.y + r.h)
                .map(|y| sf.row(y)[r.x..r.x + r.w].iter().map(surface::out_cell).collect())
                .collect();
            let dirty_rows = match dirty {
                None => (0..r.h).collect(),
                Some(d) => d.iter().filter(|&&y| y >= r.y && y < r.y + r.h).map(|y| y - r.y).collect(),
            };
            let cursor = sf.cursor.filter(|c| p.focused && c.x >= r.x && c.x < r.x + r.w && c.y >= r.y && c.y < r.y + r.h)
                .map(|c| CursorOut { x: c.x - r.x, y: c.y - r.y, visible: c.visible, shape: if c.shape == 0 { "block" } else { "other" } })
                .unwrap_or(CursorOut { x: 0, y: 0, visible: false, shape: "block" });
            PaneSurface { pane_id: p.id.clone(), x: r.x, y: r.y, cols: r.w, rows: r.h, focused: p.focused, cursor, rows_data, dirty_rows }
        })
        .collect()
}

fn emit_surface(events: &UnboundedSender<Event>, id: &str, tab: &str, sf: &Surface, dirty: Option<&[usize]>) {
    let panes = pane_surfaces(sf, dirty);
    if dirty.is_some() && panes.iter().all(|p| p.dirty_rows.is_empty()) && sf.cursor.is_none() {
        return;
    }
    let _ = events.send(Event::Surface { machine_id: id.into(), tab_id: tab.into(), panes });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_sequence() {
        let v: Vec<u64> = (0..8).map(backoff_secs).collect();
        assert_eq!(v, vec![1, 2, 4, 8, 16, 30, 30, 30]);
    }
}
