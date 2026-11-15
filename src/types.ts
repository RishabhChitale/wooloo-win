// Types mirroring INTERFACE.md (core <-> frontend contract).

export type RGB = [number, number, number];

export interface Machine {
  id: string;
  label: string;
  kind: "local" | "ssh";
  ssh_target?: string;
  session?: string;
  pipe_path?: string;
}

export interface MachineStatus {
  machine_id: string;
  state: "connected" | "connecting" | "failed";
  reason?: string;
  /** Paths/addresses the core tried (optional, added by the Rust side). */
  tried?: string[];
  /** Short suggested next step (optional). */
  hint?: string;
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
  /** Tombstone: deleted Spaces are included in get_meta(); the UI hides them. */
  deleted: boolean;
  /** Per-field hybrid logical clock of the last write. */
  hlc: Record<string, { wall: number; counter: number }>;
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
  deleted?: boolean;
}
/** Argument of set_space_meta: ONE object, camelCase keys; returns the full merged Space. */
export interface SetSpaceMetaArgs { machineId: string; herdrId: string; fields: SpaceMetaPatch; uuid?: string }
/** sync_now(machine_id) result. */
export interface SyncReport { peer: string; sent: number; received: number; stored: number; rejected: number; peer_accepted: number; peer_rejected: number }
/** sync_status() entry / event "sync-status", one per paired (ssh) machine. */
export interface SyncStatus {
  machine_id: string;
  peer_label: string;
  state: "connecting" | "ok" | "offline" | "skew";
  /** Epoch ms UTC of the last successful round; null/absent = never. */
  last_ok_ms?: number | null;
  /** Local ops the peer has not acked yet. */
  pending: number;
  local_herdr_version: string | null;
  peer_herdr_version: string | null;
  syncing: boolean;
  last_error?: string | null;
  last_report?: SyncReport | null;
}
/** Event "meta-changed". */
export interface MetaChanged { machine_id: string; source: "local" | "sync" }

/** test_connection (INTERFACE.md): one hop of the connectivity check. */
export interface DiagStep { step: "tailscale" | "tcp" | "ssh" | "bridge" | "reverse"; label: string; status: "ok" | "fail" | "skip"; detail: string }
export interface DiagReport { steps: DiagStep[]; ok: boolean; hint: string | null }
