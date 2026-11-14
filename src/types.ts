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

// ---- Synced context (frontend side of the Tauri commands set_space_meta / get_meta / sync_now / sync_status).
// Mirrors docs/sync-protocol.md section 4.1. Until INTERFACE.md carries the final spec this is the assumed shape.
/** Replicated Space metadata, already merged (LWW) by the Rust side. `machine_id` + `herdr_id` locate the workspace. */
export interface SpaceMeta {
  uuid: string;
  machine_id: string;
  herdr_id: string;
  name: string;
  category: string | null;
  color: string | null;
  order: number;
  notes: string | null;
  deleted?: boolean;
}
/** Fields the UI may write; `null` clears category/color/notes. */
export interface SpaceMetaPatch {
  name?: string;
  category?: string | null;
  color?: string | null;
  order?: number;
  notes?: string | null;
}
/** Per paired machine. */
export interface SyncStatus {
  machine_id: string;
  peer_label: string;
  state: "synced" | "syncing" | "offline";
  last_sync_secs_ago?: number;
  pending: number;
  local_herdr_version?: string;
  peer_herdr_version?: string;
}
