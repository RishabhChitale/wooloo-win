//! Tauri glue for Space metadata sync (wooloo_core::meta). Blocking work runs on spawn_blocking.
use crate::{s, App, Res};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};
use wooloo_core::machines::MachineKind;
use wooloo_core::meta::{valid_alias, MetaStore, SetMeta, SyncReport};

/// Last sync result per machine, for `sync_status`.
#[derive(Clone, Debug, Default, Serialize)]
pub struct MachineSync {
    pub machine_id: String,
    pub syncing: bool,
    pub last_ok_ms: Option<u64>,
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

#[derive(Deserialize)]
pub struct SetSpaceMeta {
    pub uuid: Option<String>,
    pub herdr_id: Option<String>,
    pub host: Option<String>,
    #[serde(default)]
    pub fields: Map<String, Value>,
}

/// Edit a Space's metadata locally. Returns the Space uuid. Emits `meta-changed` with this machine's node id.
#[tauri::command(rename_all = "snake_case")]
pub async fn set_space_meta(app: AppHandle, st: State<'_, App>, req: SetSpaceMeta) -> Res<String> {
    let store = st.sync.store.clone();
    let (uuid, node) = blocking(move || {
        let uuid = store
            .set_space_meta(SetMeta { uuid: req.uuid, herdr_id: req.herdr_id, host: req.host, fields: req.fields })
            .map_err(s)?;
        Ok((uuid, store.node().map_err(s)?))
    })
    .await?;
    emit_changed(&app, &node, "local");
    Ok(uuid)
}

/// `{node, spaces:[{uuid, host, herdr_id, name, category, color, order, notes, pinned_tabs}]}`.
#[tauri::command]
pub async fn get_meta(st: State<'_, App>) -> Res<Value> {
    let store = st.sync.store.clone();
    blocking(move || store.get_meta().map_err(s)).await
}

/// Sync with one paired machine now. Returns that machine's status record.
#[tauri::command(rename_all = "snake_case")]
pub async fn sync_now(app: AppHandle, st: State<'_, App>, machine_id: String) -> Res<MachineSync> {
    let alias = {
        let f = st.file.lock().unwrap();
        match f.get(&machine_id) {
            Some(m) => match (&m.kind, m.ssh_target.as_deref()) {
                (MachineKind::Ssh, Some(t)) if valid_alias(t) => t.to_owned(),
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
    let store = sync.store.clone();
    let res = blocking(move || store.sync_ssh(&alias).map_err(s)).await;
    let out = {
        let mut g = sync.status.lock().unwrap();
        let e = g.entry(machine_id.clone()).or_default();
        e.machine_id = machine_id.clone();
        e.syncing = false;
        match &res {
            Ok(r) => {
                e.last_ok_ms = Some(wooloo_sync::now_ms());
                e.last_error = None;
                e.last_report = Some(r.clone());
            }
            Err(m) => e.last_error = Some(m.clone()),
        }
        e.clone()
    };
    if matches!(&res, Ok(r) if r.changed()) {
        emit_changed(&app, &machine_id, "sync");
    }
    res.map(|_| out)
}

/// Status of every machine that has been synced this run.
#[tauri::command]
pub fn sync_status(st: State<'_, App>) -> Vec<MachineSync> {
    let mut v: Vec<MachineSync> = st.sync.status.lock().unwrap().values().cloned().collect();
    v.sort_by(|a, b| a.machine_id.cmp(&b.machine_id));
    v
}
