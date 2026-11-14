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
