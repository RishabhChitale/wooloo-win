//! Thin Tauri glue over wooloo-core. Commands and events follow INTERFACE.md.
mod pair;
mod sync;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc::unbounded_channel;
use wooloo_core::machines::{Machine, MachinesFile};
use wooloo_core::manager::{spawn_machine, Command, Event, MachineHandle};

struct App {
    path: PathBuf,
    file: Mutex<MachinesFile>,
    handles: Mutex<HashMap<String, MachineHandle>>,
    events: tokio::sync::mpsc::UnboundedSender<Event>,
    sync: sync::SyncState,
}

type Res<T> = Result<T, String>;
fn s<E: ToString>(e: E) -> String {
    e.to_string()
}

impl App {
    fn handle(&self, id: &str) -> Res<MachineHandle> {
        self.handles.lock().unwrap().get(id).cloned().ok_or_else(|| format!("unknown machine {id}"))
    }
    fn start(&self, m: &Machine) {
        // Sync commands run outside any tokio runtime; tokio::spawn would panic (and kill the app). Enter Tauri's runtime first.
        let tauri::async_runtime::RuntimeHandle::Tokio(rt) = tauri::async_runtime::handle().clone();
        let _guard = rt.enter();
        let h = spawn_machine(m.clone(), self.events.clone());
        if let Some(old) = self.handles.lock().unwrap_or_else(|e| e.into_inner()).insert(m.id.clone(), h) {
            old.send(Command::Stop);
        }
    }
}

#[tauri::command]
fn list_machines(app: State<App>) -> Vec<Machine> {
    app.file.lock().unwrap().machines.clone()
}

#[tauri::command(rename_all = "snake_case")]
fn add_machine(app: State<App>, label: String, ssh_target: String, session: Option<String>) -> Res<Machine> {
    let m = {
        let mut f = app.file.lock().unwrap();
        let m = f.add_ssh(label, ssh_target, session).map_err(s)?;
        f.save(&app.path).map_err(s)?;
        m
    };
    app.start(&m);
    Ok(m)
}

#[tauri::command]
fn remove_machine(app: State<App>, id: String) -> Res<()> {
    {
        let mut f = app.file.lock().unwrap();
        f.remove(&id);
        f.save(&app.path).map_err(s)?;
    }
    if let Some(h) = app.handles.lock().unwrap().remove(&id) {
        h.send(Command::Stop);
    }
    Ok(())
}

/// Chooses which Herdr session a machine uses (`None` = discover automatically). Saved to machines.toml,
/// then the machine restarts with it. Used when the failure reason says "found sessions: a, b; choose one".
#[tauri::command(rename_all = "snake_case")]
fn set_machine_session(app: State<App>, id: String, session: Option<String>) -> Res<Machine> {
    let session = session.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let m = {
        let mut f = app.file.lock().unwrap();
        let m = {
            let m = f.machines.iter_mut().find(|m| m.id == id).ok_or_else(|| format!("unknown machine {id}"))?;
            m.session = session;
            m.validate().map_err(s)?;
            m.clone()
        };
        f.save(&app.path).map_err(s)?;
        m
    };
    app.start(&m);
    Ok(m)
}

#[tauri::command]
fn reconnect_machine(app: State<App>, id: String) -> Res<()> {
    app.handle(&id)?.send(Command::Reconnect);
    Ok(())
}

#[tauri::command]
fn open_space(app: State<App>, machine_id: String, workspace_id: String) -> Res<()> {
    app.handle(&machine_id)?.send(Command::OpenSpace(workspace_id));
    Ok(())
}

#[tauri::command]
fn select_tab(app: State<App>, machine_id: String, tab_id: String) -> Res<()> {
    app.handle(&machine_id)?.send(Command::SelectTab(tab_id));
    Ok(())
}

#[tauri::command]
fn focus_pane(app: State<App>, machine_id: String, pane_id: String) -> Res<()> {
    app.handle(&machine_id)?.send(Command::FocusPane(pane_id));
    Ok(())
}

#[derive(Deserialize)]
struct KeyIn {
    text: Option<String>,
    code: String,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    alt: bool,
    #[serde(default)]
    shift: bool,
}

#[tauri::command]
fn send_key(app: State<App>, machine_id: String, pane_id: String, key: KeyIn) -> Res<()> {
    app.handle(&machine_id)?.send(Command::Key {
        pane_id,
        text: key.text,
        code: key.code,
        ctrl: key.ctrl,
        alt: key.alt,
        shift: key.shift,
    });
    Ok(())
}

#[tauri::command]
fn send_paste(app: State<App>, machine_id: String, pane_id: String, text: String) -> Res<()> {
    app.handle(&machine_id)?.send(Command::Paste { pane_id, text });
    Ok(())
}

#[tauri::command]
fn resize_viewport(app: State<App>, machine_id: String, cols: u32, rows: u32) -> Res<()> {
    app.handle(&machine_id)?.send(Command::Resize { cols, rows });
    Ok(())
}

#[tauri::command]
fn scroll(app: State<App>, machine_id: String, pane_id: String, lines: i32) -> Res<()> {
    app.handle(&machine_id)?.send(Command::Scroll { pane_id, lines });
    Ok(())
}

/// True if `herdr status server` output reports a running server (`status: running`; "not running" is not).
fn status_says_running(out: &str) -> bool {
    out.lines().any(|l| {
        let l = l.trim().to_ascii_lowercase();
        l.strip_prefix("status:").map(|v| v.trim() == "running").unwrap_or(false)
    })
}

/// True if a server's early exit output means another server already owns the socket.
fn exit_says_already_running(out: &str) -> bool {
    out.to_ascii_lowercase().contains("already running")
}

fn herdr_cmd(args: &[&str]) -> std::process::Command {
    let mut c = std::process::Command::new(wooloo_core::resolve::herdr_program());
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Windows: true if the API pipe exists but refuses us (server started from an elevated window).
fn pipe_denied() -> bool {
    #[cfg(windows)]
    {
        let name = wooloo_core::transport::pipe_name(&None, wooloo_core::transport::Channel::Api, &None);
        return match std::fs::OpenOptions::new().read(true).write(true).open(&name) {
            Err(e) => wooloo_core::manager::is_access_denied_io(&e),
            Ok(_) => false,
        };
    }
    #[cfg(not(windows))]
    false
}

/// Starts this PC's own Herdr server (`herdr server`) detached, with no console window on Windows.
/// Does nothing (Ok) if a server is already running.
#[tauri::command]
fn start_local_herdr() -> Res<()> {
    if let Ok(o) = herdr_cmd(&["status", "server"]).stdin(std::process::Stdio::null()).output() {
        if o.status.success() && status_says_running(&String::from_utf8_lossy(&o.stdout)) {
            if !pipe_denied() {
                return Ok(());
            }
            // Running but its pipe denies us: stop it (best effort) and start a fresh, non-elevated one.
            let _ = herdr_cmd(&["server", "stop"]).stdin(std::process::Stdio::null()).output();
            std::thread::sleep(std::time::Duration::from_millis(700));
            if pipe_denied() {
                return Err(wooloo_core::manager::ELEVATED_SERVER_MSG.into());
            }
        }
    }
    let mut c = herdr_cmd(&["server"]);
    c.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => {
            wooloo_core::resolve::invalidate();
            let detail = match wooloo_core::resolve::resolve_herdr() {
                Ok(r) => format!("resolved {}", r.path.display()),
                Err(t) => t,
            };
            return Err(format!("could not run `herdr server`: {e}. {detail}"));
        }
    };
    std::thread::sleep(std::time::Duration::from_millis(700));
    if let Ok(Some(st)) = child.try_wait() {
        let mut err = String::new();
        if let Some(mut p) = child.stderr.take() {
            use std::io::Read;
            let _ = p.read_to_string(&mut err);
        }
        if exit_says_already_running(&err) {
            return Ok(());
        }
        return Err(format!("`herdr server` exited ({st}): {}", err.trim()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_running_detection() {
        assert!(status_says_running("status: running\npid: 4\n"));
        assert!(status_says_running("Status:   Running\r\n"));
        assert!(!status_says_running("status: not running\n"));
        assert!(!status_says_running("status: stopped"));
        assert!(!status_says_running(""));
    }

    /// Real `herdr status server` output (Mac, herdr 0.9.5, exit 0).
    const STATUS_RUNNING_0_9_5: &str =
        "status: running\nversion: 0.9.5\nendpoint_compatible: yes\nprivate_protocol: 22\nprivate_protocol_compatible: yes\n";

    #[test]
    fn status_running_real_output_fixture() {
        assert!(status_says_running(STATUS_RUNNING_0_9_5));
    }

    #[test]
    fn status_not_running_cases() {
        // Stopped-server text is not known verbatim; any output without `status: running` must be false.
        assert!(!status_says_running(""));
        assert!(!status_says_running("version: 0.9.5\nendpoint_compatible: yes\n"));
        assert!(!status_says_running("herdr server is not running\n"));
    }

    #[test]
    fn already_running_exit() {
        assert!(exit_says_already_running("Error: herdr server is Already Running (pid 3)"));
        assert!(!exit_says_already_running("permission denied"));
    }
}

fn forward(app: AppHandle, ev: Event) {
    let _ = match ev {
        Event::Status { machine_id, state, reason, retry_in_secs, tried, hint } => app.emit(
            "machine-status",
            json!({"machine_id": machine_id, "state": state, "reason": reason, "retry_in_secs": retry_in_secs, "tried": tried, "hint": hint}),
        ),
        Event::Snapshot { machine_id, workspaces } => {
            app.emit("snapshot", json!({"machine_id": machine_id, "workspaces": workspaces}))
        }
        Event::Surface { machine_id, tab_id, panes } => {
            app.emit("surface", json!({"machine_id": machine_id, "tab_id": tab_id, "panes": panes}))
        }
    };
}

/// Where panics are logged: %APPDATA%\wooloo\crash.log (or $HOME/.wooloo/crash.log).
fn crash_log_path() -> PathBuf {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("wooloo")).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".wooloo"))).unwrap_or_else(std::env::temp_dir).join("crash.log")
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let p = crash_log_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let line = format!("[{ts}] panic: {info}\n{}\n", std::backtrace::Backtrace::force_capture());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
            use std::io::Write;
            let _ = f.write_all(line.as_bytes());
        }
        prev(info);
    }));
}

pub fn run() {
    install_panic_hook();
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("machines.toml");
            let file = MachinesFile::load(&path).map_err(|e| e.to_string())?;
            let (tx, mut rx) = unbounded_channel::<Event>();
            let state = App { path, file: Mutex::new(file.clone()), handles: Mutex::new(HashMap::new()), events: tx, sync: sync::SyncState::new(wooloo_core::meta::MetaStore::in_dir(&dir)) };
            // Handles are created inside the tauri async runtime.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Some(ev) = rx.recv().await {
                    forward(handle.clone(), ev);
                }
            });
            let machines = file.machines.clone();
            app.manage(state);
            let st = app.state::<App>();
            tauri::async_runtime::block_on(async {
                for m in &machines {
                    st.start(m);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_machines, add_machine, remove_machine, set_machine_session, reconnect_machine, open_space, select_tab,
            focus_pane, send_key, send_paste, resize_viewport, scroll, start_local_herdr,
            pair::pair_defaults, pair::pair_local_info, pair::pair_review, pair::pair_accept,
            sync::set_space_meta, sync::get_meta, sync::sync_now, sync::sync_status
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            let p = crash_log_path();
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&p, format!("fatal: {e}\n"));
        });
}
