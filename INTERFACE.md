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
- reconnect_machine(id)
- open_space(machine_id, workspace_id)       // subscribe terminal endpoint for that Space's selected tab
- select_tab(machine_id, tab_id)
- focus_pane(machine_id, pane_id)
- send_key(machine_id, pane_id, key:{text?, code, ctrl, alt, shift, meta})   // semantic key; Rust encodes to Herdr shell.input.semantic.v1
- send_paste(machine_id, pane_id, text)
- resize_viewport(machine_id, cols, rows)
- scroll(machine_id, pane_id, lines)
- copy text is done in the frontend (navigator.clipboard)

## Tauri events (Rust -> frontend, via app.emit)
- "machine-status"  {machine_id, state:"connected"|"connecting"|"failed", reason?, retry_in_secs?}
- "snapshot"        {machine_id, workspaces:[{id,label,category?,tabs:[{id,label,panes:[{id,...}]}], agent_state?}], ...}  (JSON passed through from herdr session.snapshot, lightly normalised)
- "surface"         {machine_id, tab_id, panes:[{pane_id, x,y,cols,rows, cursor:{x,y,visible,shape}, rows_data:[Cell[]] | patch}] }
  Cell = {ch:string, fg:[r,g,b]|null, bg:[r,g,b]|null, bold,italic,underline,inverse,wide}
  Per pane: `cursor` is always present ({x,y,visible,shape:string}; shape "block" for 0, "other" otherwise, pane-local coords). `rows_data` is the pane's full grid (rows of Cell[], one Cell per column). `dirty_rows` is per pane (pane-local row indices that changed; absent/empty on a full surface). There is no top-level dirty_rows.
  Wide cells: a double-width glyph appears in column c as a Cell with wide:true and ch = the glyph; column c+1 is a spacer Cell with ch:"" (empty), wide:false. Rows keep one Cell per column, so indices stay aligned. Renderers draw the glyph across two columns and draw nothing for the spacer. (Verified on a real stream: fixtures frame_007, test wide_cell_is_followed_by_empty_spacer.) Note `wide` comes from a codepoint-range check, not from the wire.
Exact field names may be refined by the core pawn; if changed, update this file in the same commit.

Note (frontend): `scroll(machine_id, pane_id, lines)` — positive `lines` = toward older output (wheel up), negative = toward newer.
