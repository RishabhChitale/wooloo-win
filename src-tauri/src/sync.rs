//! Tauri glue for Space metadata sync (wooloo_core::meta). Blocking work runs on spawn_blocking.
use crate::{s, App, Res};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, State};
use wooloo_core::machines::MachineKind;
use wooloo_core::meta::{valid_alias, MetaStore, SetMeta, SyncReport};

/// Last sync result per machine (internal; the frontend sees [`SyncStatus`]).
#[derive(Clone, Debug, Default)]
pub struct MachineSync {
    pub machine_id: String,
    pub syncing: bool,
    pub last_ok_ms: Option<u64>,
    pub last_error: Option<String>,
    pub last_report: Option<SyncReport>,
    /// Peer's Herdr version from the last successful sync hello (None = never learned / old peer).
    pub peer_herdr_version: Option<String>,
}

/// This machine's Herdr version (`herdr --version`), read once and cached. None = herdr missing or
/// an unusable version string. Call from a blocking context.
pub fn local_herdr_version() -> Option<String> {
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| {
        let o = std::process::Command::new("herdr").arg("--version").output().ok()?;
        if !o.status.success() {
            return None;
        }
        let t = String::from_utf8_lossy(&o.stdout);
        let v = t.lines().next()?.trim().trim_start_matches("herdr").trim().to_string();
        wooloo_sync::exchange::is_herdr_version(&v).then_some(v)
    })
    .clone()
}

/// What `sync_status()` returns and the `sync-status` event carries.
#[derive(Clone, Debug, Serialize)]
pub struct SyncStatus {
    pub machine_id: String,
    pub peer_label: String,
    /// "connecting" | "ok" | "offline" | "skew"
    pub state: &'static str,
    /// Epoch milliseconds UTC of the last successful round; null = never.
    pub last_ok_ms: Option<u64>,
    /// Local ops the peer has not acked yet.
    pub pending: u64,
    pub local_herdr_version: Option<String>,
    pub peer_herdr_version: Option<String>,
    pub syncing: bool,
    pub last_error: Option<String>,
    pub last_report: Option<SyncReport>,
}

#[derive(Clone)]
pub struct SyncState {
    pub store: MetaStore,
    pub status: Arc<Mutex<HashMap<String, MachineSync>>>,
}

impl SyncState {
    pub fn new(store: MetaStore) -> Self {
        SyncState { store, status: Arc::new(Mutex::new(HashMap::new())) }
    }
}

fn emit_changed(app: &AppHandle, machine_id: &str, source: &str) {
    let _ = app.emit("meta-changed", json!({"machine_id": machine_id, "source": source}));
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(s)?
}

/// Edit a Space's metadata. Args (camelCase, one flat object): `{machineId, herdrId, fields, uuid?}`.
/// `machineId` picks the machine that owns the Herdr server; the stored host is that machine's node id.
/// Returns the full merged Space record (with per-field hlc). Emits `meta-changed` (source "local").
#[tauri::command]
pub async fn set_space_meta(
    app: AppHandle,
    st: State<'_, App>,
    machine_id: String,
    herdr_id: Option<String>,
    fields: Option<Map<String, Value>>,
    uuid: Option<String>,
) -> Res<Value> {
    let local = {
        let f = st.file.lock().unwrap();
        match f.get(&machine_id) {
            Some(m) => matches!(m.kind, MachineKind::Local),
            None => return Err(format!("unknown machine {machine_id}")),
        }
    };
    let store = st.sync.store.clone();
    let (rec, node) = blocking(move || {
        let host = if local {
            None
        } else {
            Some(store.peer_node(&machine_id).ok_or_else(|| format!("{machine_id} has not synced yet; sync first"))?)
        };
        let rec = store
            .set_space_meta(SetMeta { uuid, herdr_id, host, fields: fields.unwrap_or_default() })
            .map_err(s)?;
        Ok((rec, store.node().map_err(s)?))
    })
    .await?;
    emit_changed(&app, &node, "local");
    Ok(rec)
}

/// `{node, spaces:[Space]}` incl. tombstones (`deleted: true`), see INTERFACE.md.
#[tauri::command]
pub async fn get_meta(st: State<'_, App>) -> Res<Value> {
    let store = st.sync.store.clone();
    blocking(move || store.get_meta().map_err(s)).await
}

fn state_of(m: &MachineSync) -> &'static str {
    if m.syncing {
        "connecting"
    } else if m.last_error.is_some() {
        "offline"
    } else if m.last_report.as_ref().is_some_and(|r| r.skewed > 0) {
        "skew"
    } else if m.last_ok_ms.is_some() {
        "ok"
    } else {
        "offline"
    }
}

/// Build one machine's status (reads the pending count from disk, so call from a blocking context).
fn build_status(sync: &SyncState, machine_id: &str, label: &str) -> SyncStatus {
    let m = sync.status.lock().unwrap().get(machine_id).cloned().unwrap_or_default();
    SyncStatus {
        machine_id: machine_id.to_owned(),
        peer_label: label.to_owned(),
        state: state_of(&m),
        last_ok_ms: m.last_ok_ms,
        pending: sync.store.pending(machine_id).unwrap_or(0),
        local_herdr_version: local_herdr_version(),
        peer_herdr_version: m.peer_herdr_version,
        syncing: m.syncing,
        last_error: m.last_error,
        last_report: m.last_report,
    }
}

async fn status_of(app: &AppHandle, sync: &SyncState, machine_id: &str, label: &str) -> SyncStatus {
    let (sy, id, lb) = (sync.clone(), machine_id.to_owned(), label.to_owned());
    let st = blocking(move || Ok(build_status(&sy, &id, &lb))).await.unwrap_or_else(|_| SyncStatus {
        machine_id: machine_id.to_owned(),
        peer_label: label.to_owned(),
        state: "offline",
        last_ok_ms: None,
        pending: 0,
        local_herdr_version: None,
        peer_herdr_version: None,
        syncing: false,
        last_error: None,
        last_report: None,
    });
    let _ = app.emit("sync-status", &st);
    st
}

/// Sync with one paired machine now. Returns that machine's status. Emits `sync-status` when the
/// state changes (connecting, then ok/offline/skew) and `meta-changed` when data arrived.
#[tauri::command(rename_all = "snake_case")]
pub async fn sync_now(app: AppHandle, st: State<'_, App>, machine_id: String) -> Res<SyncStatus> {
    let (alias, label) = {
        let f = st.file.lock().unwrap();
        match f.get(&machine_id) {
            Some(m) => match (&m.kind, m.ssh_target.as_deref()) {
                (MachineKind::Ssh, Some(t)) if valid_alias(t) => (t.to_owned(), m.label.clone()),
                _ => return Err("sync needs a paired machine".into()),
            },
            None => return Err(format!("unknown machine {machine_id}")),
        }
    };
    let sync = st.sync.clone();
    {
        let mut g = sync.status.lock().unwrap();
        let e = g.entry(machine_id.clone()).or_insert_with(|| MachineSync { machine_id: machine_id.clone(), ..Default::default() });
        if e.syncing {
            return Err("sync already running".into());
        }
        e.syncing = true;
    }
    status_of(&app, &sync, &machine_id, &label).await; // connecting
    let store = sync.store.clone();
    let mid = machine_id.clone();
    let res = blocking(move || {
        let lv = local_herdr_version();
        let r = store.sync_ssh_with(&alias, lv.as_deref()).map_err(s)?;
        store.note_synced(&mid, &r).map_err(s)?;
        Ok(r)
    })
    .await;
    {
        let mut g = sync.status.lock().unwrap();
        let e = g.entry(machine_id.clone()).or_default();
        e.machine_id = machine_id.clone();
        e.syncing = false;
        match &res {
            Ok(r) => {
                e.last_ok_ms = Some(wooloo_sync::now_ms());
                e.last_error = None;
                e.peer_herdr_version = r.peer_herdr_version.clone();
                e.last_report = Some(r.clone());
            }
            Err(m) => e.last_error = Some(m.clone()),
        }
    }
    if matches!(&res, Ok(r) if r.changed()) {
        emit_changed(&app, &machine_id, "sync");
    }
    let out = status_of(&app, &sync, &machine_id, &label).await;
    res.map(|_| out)
}

/// Status of every paired (ssh) machine; `state` is "offline" until its first successful round.
#[tauri::command]
pub async fn sync_status(st: State<'_, App>) -> Res<Vec<SyncStatus>> {
    let machines: Vec<(String, String)> = {
        let f = st.file.lock().unwrap();
        f.machines.iter().filter(|m| matches!(m.kind, MachineKind::Ssh)).map(|m| (m.id.clone(), m.label.clone())).collect()
    };
    let sync = st.sync.clone();
    let mut v = blocking(move || Ok(machines.iter().map(|(i, l)| build_status(&sync, i, l)).collect::<Vec<_>>())).await?;
    v.sort_by(|a, b| a.machine_id.cmp(&b.machine_id));
    Ok(v)
}
