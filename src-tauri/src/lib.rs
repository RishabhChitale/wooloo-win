//! Thin Tauri glue over wooloo-core. Commands and events follow INTERFACE.md.
mod pair;
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
        let h = spawn_machine(m.clone(), self.events.clone());
        if let Some(old) = self.handles.lock().unwrap().insert(m.id.clone(), h) {
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

fn forward(app: AppHandle, ev: Event) {
    let _ = match ev {
        Event::Status { machine_id, state, reason, retry_in_secs } => app.emit(
            "machine-status",
            json!({"machine_id": machine_id, "state": state, "reason": reason, "retry_in_secs": retry_in_secs}),
        ),
        Event::Snapshot { machine_id, workspaces } => {
            app.emit("snapshot", json!({"machine_id": machine_id, "workspaces": workspaces}))
        }
        Event::Surface { machine_id, tab_id, panes } => {
            app.emit("surface", json!({"machine_id": machine_id, "tab_id": tab_id, "panes": panes}))
        }
    };
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("machines.toml");
            let file = MachinesFile::load(&path).map_err(|e| e.to_string())?;
            let (tx, mut rx) = unbounded_channel::<Event>();
            let state = App { path, file: Mutex::new(file.clone()), handles: Mutex::new(HashMap::new()), events: tx };
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
            list_machines, add_machine, remove_machine, reconnect_machine, open_space, select_tab,
            focus_pane, send_key, send_paste, resize_viewport, scroll,
            pair::pair_local_info, pair::pair_review, pair::pair_accept
        ])
        .run(tauri::generate_context!())
        .expect("error while running wooloo");
}
