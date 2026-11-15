# Contract between wooloo-core / src-tauri (Rust) and the frontend (TS)

Layout (monorepo, git root ~/Claude/wooloo-win):
- crates/wooloo-core/   Rust lib: protocol, decoder, machines.toml, connection manager (no Tauri dependency)
- src-tauri/            Tauri v2 app; thin glue exposing wooloo-core via commands + events
- src/                  Vite + vanilla TypeScript frontend
- .github/workflows/    ci.yml, release.yml;  README.md

## Tauri commands (frontend invoke(), all async, never block on a dead machine)
- list_machines() -> Machine[]        Machine {id, label, kind:"local"|"ssh", ssh_target?, session?}
- add_machine({label, ssh_target, session?}) -> Machine   // args are snake_case (unlike the other commands, which use camelCase keys). Machine also has optional `pipe_path` (local Windows API pipe override, set in machines.toml)
- remove_machine(id)
- set_machine_session({id, session?}) -> Machine   // snake_case keys. Saves the machine's Herdr session to machines.toml (null/empty = auto-discover) and restarts it. Local machines with no session discover their own running sessions on every attempt (`herdr session list --json`, then `\\.\pipe\` names matching herdr*): default session if running, else the only running one, else Failed with reason "found sessions: a, b; choose one" and a hint naming set_machine_session. "default" means the unnamed root session.
- Herdr executable: every herdr call (session list, status server, server, --version, pairing, authorized_keys `--herdr`) uses the absolute path from wooloo_core::resolve::resolve_herdr(): registry Path (HKCU then HKLM, expanded) -> installer dirs (`%LOCALAPPDATA%\Programs\Herdr\bin`, `<HERDR_HOME|%USERPROFILE%\.herdr>\packages\standalone\current`, HERDR_INSTALL_DIR) -> `where.exe herdr.exe` -> process PATH. Cached; re-resolved when the file vanishes or a spawn fails. Errors list every dir/value tried. pair_local_info and pair_review responses carry `herdr_path` (string|null) for display.
- reconnect_machine(id)       // restarts the attempt now and resets the retry delay to 1 s
- test_connection(id) -> DiagReport   // read-only hop check of an SSH machine, stops at the first fail. `{steps:[{step:"tailscale"|"tcp"|"ssh"|"bridge"|"reverse", label, status:"ok"|"fail"|"skip", detail}], ok, hint|null}`. Steps (plus an informational "reverse" skip step after a tcp/ssh failure naming the command to run on the peer): tailscale ping (skip if no CLI), TCP connect to the resolved ssh host:port (5 s), `ssh -o BatchMode=yes <target> true` (a paired peer's dispatcher rejecting `true` after login counts as ok), then a `session.snapshot` over the real bridge argv (10 s). Local machines: error. Defined in src-tauri/src/diag.rs.
- start_local_herdr() -> void       // spawns `herdr server` detached (hidden window on Windows); errors as a string. The machine reconnects on its own retry or reconnect_machine(id).
- open_space(machine_id, workspace_id)       // subscribe terminal endpoint for that Space's selected tab
- select_tab(machine_id, tab_id)
- focus_pane(machine_id, pane_id)
- send_key(machine_id, pane_id, key:{text?, code, ctrl, alt, shift, meta})   // semantic key; Rust encodes to Herdr shell.input.semantic.v1
- send_paste(machine_id, pane_id, text)
- resize_viewport(machine_id, cols, rows)
- scroll(machine_id, pane_id, lines)
- set_space_meta({machineId, herdrId, fields, uuid?}) -> Space   // ONE object arg, camelCase keys. fields: name, category, color, order, notes, pinned_tabs, deleted (allow-listed only). Returns the full merged Space record (see get_meta) with per-field hlc so the UI updates without a refetch. Emits meta-changed.
- get_meta() -> {node, spaces:[Space]}
  Space = {uuid, host, herdr_id, name, category, color, order, notes, pinned_tabs, deleted:boolean, hlc:{<field>:{wall,counter}}}  (hlc per field, incl. `deleted`)
  `host` = the stable NODE ID of the machine that owns the Herdr server (local = this app's own node id; a paired ssh machine = the peer's node id, learned at pairing / first sync handshake). Never a user-editable label.
  Tombstones: deleted Spaces are INCLUDED with deleted:true (and the deleted hlc) so they stay deleted across peers; the UI hides them.
- sync_now(machine_id) -> SyncReport {peer,sent,received,stored,rejected,peer_accepted,peer_rejected}   // ssh machines only; runs ssh to the paired `wooloo-peer-<id>` alias with the allow-listed request `wooloo-peer sync`
- sync_status() -> SyncStatus[] {machine_id, peer_label, state:"connecting"|"ok"|"offline"|"skew", last_ok_ms, pending, local_herdr_version, peer_herdr_version, syncing, last_error?, last_report?}
  last_ok_ms is epoch MILLISECONDS UTC (null/absent = never synced). pending = count of local ops not yet acked by that peer. Herdr versions are strings (`herdr --version`, e.g. "0.9.5"); null = unknown. local_herdr_version is read once from `herdr --version` and cached (null if herdr is missing). peer_herdr_version comes from the sync hello (spec s8.1) and is kept from the last successful round; null = never synced yet or an old peer that does not send it. SyncReport also carries `peer_herdr_version`.
  Replica is persisted in the app config dir as meta-state.json. Allow-list (docs/peer-allowlist.json) is at revision 2 (new entry wooloo-sync).
- Crash safety: `pair_accept` catches panics and returns them as an error string (shown in the Pair dialog); a global panic hook appends to `%APPDATA%\wooloo\crash.log`. Starting a machine from a sync command enters Tauri's tokio runtime first (spawning outside it used to panic and close the app on Accept pairing).
- copy text is done in the frontend (navigator.clipboard)

## Tauri events (Rust -> frontend, via app.emit)
- "machine-status"  {machine_id, state:"connected"|"connecting"|"failed", reason?, retry_in_secs?, tried?:string[], hint?:string}
  Failed reasons: local, no server: "no Herdr server found: tried <pipe/socket path>; start it with: herdr" (hint:"herdr"; show a Start Herdr button -> start_local_herdr()); local, endpoint reached: "connected to <path> but handshake failed: <err>"; local, pipe refused (os error 5 / PermissionDenied): "The Herdr server was started by a different (elevated) user context. Stop it with: herdr server stop (in the window where you started it) and click Start Herdr" (hint:"herdr server stop"; start_local_herdr also stops such a server itself); ssh: first line of ssh stderr + "(ssh <target>)". `tried` = endpoints attempted (empty when not failed); `hint` is a command the user can run, absent if none.
- "meta-changed"    {machine_id, source:"local"|"sync"}   // Space metadata replica changed (machine_id = own node id for local edits)
- "sync-status"     SyncStatus entry (same shape as sync_status() items)   // emitted on every state change (connecting/ok/offline/skew) and after each sync round
- "snapshot"        {machine_id, workspaces:[{id,label,category?,presence?:string[],tabs:[{id,label,panes:[{id,...}]}], agent_state?}], ...}  (JSON passed through from herdr session.snapshot, lightly normalised)
- "surface"         {machine_id, tab_id, panes:[{pane_id, x,y,cols,rows, cursor:{x,y,visible,shape}, rows_data:[Cell[]] | patch}] }
  Cell = {ch:string, fg:[r,g,b]|null, bg:[r,g,b]|null, bold,italic,underline,inverse,wide}
  Per pane: `cursor` is always present ({x,y,visible,shape:string}; shape "block" for 0, "other" otherwise, pane-local coords). `rows_data` is the pane's full grid (rows of Cell[], one Cell per column). `dirty_rows` is per pane (pane-local row indices that changed; absent/empty on a full surface). There is no top-level dirty_rows.
  Wide cells: a double-width glyph appears in column c as a Cell with wide:true and ch = the glyph; column c+1 is a spacer Cell with ch:"" (empty), wide:false. Rows keep one Cell per column, so indices stay aligned. Renderers draw the glyph across two columns and draw nothing for the spacer. (Verified on a real stream: fixtures frame_007, test wide_cell_is_followed_by_empty_spacer.) Note `wide` comes from a codepoint-range check, not from the wire.
Exact field names may be refined by the core pawn; if changed, update this file in the same commit.

Note (frontend): `scroll(machine_id, pane_id, lines)` — positive `lines` = toward older output (wheel up), negative = toward newer.
