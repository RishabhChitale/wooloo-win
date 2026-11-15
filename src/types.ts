// Types mirroring INTERFACE.md (core <-> frontend contract).

export type RGB = [number, number, number];

export interface Machine {
  id: string;
  label: string;
  kind: "local" | "ssh";
  ssh_target?: string;
  session?: string;
}

export interface MachineStatus {
  machine_id: string;
  state: "connected" | "connecting" | "failed";
  reason?: string;
  retry_in_secs?: number;
}

export interface PaneRef { id: string; [k: string]: unknown }
export interface Tab { id: string; label: string; panes: PaneRef[] }
export interface Workspace {
  /** Machines (labels) that have this Space attached; optional snapshot field. */
  presence?: string[];
  id: string;
  label: string;
  category?: string;
  tabs: Tab[];
  agent_state?: string;
}
export interface Snapshot { machine_id: string; workspaces: Workspace[]; [k: string]: unknown }

export interface Cell {
  ch: string;
  fg: RGB | null;
  bg: RGB | null;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  inverse: boolean;
  wide: boolean;
}

export interface Cursor { x: number; y: number; visible: boolean; shape?: string }

export interface PaneSurface {
  pane_id: string;
  x: number; y: number; cols: number; rows: number;
  cursor: Cursor;
  rows_data: Cell[][];
  dirty_rows?: number[];
}
export interface SurfaceEvent { machine_id: string; tab_id: string; panes: PaneSurface[] }

export interface KeyInput {
  text?: string;
  code: string;
  ctrl: boolean; alt: boolean; shift: boolean; meta: boolean;
}

export type Unlisten = () => void;

// ---- Synced context: mirrors INTERFACE.md (set_space_meta / get_meta / sync_now / sync_status, event "meta-changed").
/** One entry of get_meta().spaces. `host` + `herdr_id` locate the workspace; `uuid` is the cross-machine identity. */
export interface SpaceMeta {
  uuid: string;
  host: string;
  herdr_id: string;
  name: string;
  category: string | null;
  color: string | null;
  order: number;
  notes: string | null;
  pinned_tabs: string[];
}
/** get_meta() result. `node` is this app's own node id (the `host` of local Spaces). */
export interface MetaReplica { node: string; spaces: SpaceMeta[] }
/** `fields` of set_space_meta (allow-listed only); `null` clears category/color/notes. */
export interface SpaceMetaPatch {
  name?: string;
  category?: string | null;
  color?: string | null;
  order?: number;
  notes?: string | null;
  pinned_tabs?: string[];
}
/** Argument of set_space_meta. */
export interface SetSpaceMetaArgs { uuid?: string; herdr_id?: string; host: string; fields: SpaceMetaPatch }
/** sync_now(machine_id) result. */
export interface SyncReport { peer: string; sent: number; received: number; stored: number; rejected: number; peer_accepted: number; peer_rejected: number }
/** sync_status() entry, one per paired (ssh) machine. */
export interface MachineSync { machine_id: string; syncing: boolean; last_ok_ms?: number; last_error?: string; last_report?: SyncReport }
/** Event "meta-changed". */
export interface MetaChanged { machine_id: string; source: "local" | "sync" }
