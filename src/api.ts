import type { Cell, KeyInput, Machine, MachineStatus, PaneSurface, Snapshot, SyncStatus, MetaChanged, MetaReplica, SetSpaceMetaArgs, SpaceMeta, SurfaceEvent, SyncReport, Unlisten } from "./types";

// Thin wrapper over Tauri invoke/listen (INTERFACE.md). Outside Tauri (pnpm dev in a browser)
// a mock backend emits fake machines/spaces/surfaces so the UI works without Rust.

export interface Api {
  listMachines(): Promise<Machine[]>;
  addMachine(m: { label: string; ssh_target: string; session?: string; pipe_path?: string }): Promise<Machine>;
  removeMachine(id: string): Promise<void>;
  reconnectMachine(id: string): Promise<void>;
  /** Start a local Herdr server (local machines only). */
  startLocalHerdr(id: string): Promise<void>;
  openSpace(machineId: string, workspaceId: string): Promise<void>;
  selectTab(machineId: string, tabId: string): Promise<void>;
  focusPane(machineId: string, paneId: string): Promise<void>;
  sendKey(machineId: string, paneId: string, key: KeyInput): Promise<void>;
  sendPaste(machineId: string, paneId: string, text: string): Promise<void>;
  resizeViewport(machineId: string, cols: number, rows: number): Promise<void>;
  scroll(machineId: string, paneId: string, lines: number): Promise<void>;
  // Synced context, exactly as in INTERFACE.md.
  getMeta(): Promise<MetaReplica>;
  setSpaceMeta(a: SetSpaceMetaArgs): Promise<SpaceMeta>;
  syncNow(machineId: string): Promise<SyncReport>;
  syncStatus(): Promise<SyncStatus[]>;
  onSyncStatus(cb: (s: SyncStatus) => void): Promise<Unlisten>;
  onMetaChanged(cb: (e: MetaChanged) => void): Promise<Unlisten>;
  onMachineStatus(cb: (s: MachineStatus) => void): Promise<Unlisten>;
  onSnapshot(cb: (s: Snapshot) => void): Promise<Unlisten>;
  onSurface(cb: (s: SurfaceEvent) => void): Promise<Unlisten>;
}

export const inTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

const toSnake = (k: string) => k.replace(/[A-Z]/g, (m) => "_" + m.toLowerCase());
export function snakeKeys(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(snakeKeys);
  if (v && typeof v === "object") {
    return Object.fromEntries(Object.entries(v as Record<string, unknown>).map(([k, x]) => [toSnake(k), snakeKeys(x)]));
  }
  return v;
}

async function createTauriApi(): Promise<Api> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  // Event payloads follow INTERFACE.md (snake_case). Normalise camelCase keys too, so either serde setting works.
  const on = <T>(name: string) => async (cb: (p: T) => void): Promise<Unlisten> =>
    listen<unknown>(name, (e) => cb(snakeKeys(e.payload) as T));
  return {
    listMachines: () => invoke("list_machines"),
    addMachine: (m) => invoke("add_machine", m),
    removeMachine: (id) => invoke("remove_machine", { id }),
    reconnectMachine: (id) => invoke("reconnect_machine", { id }),
    startLocalHerdr: () => invoke("start_local_herdr"),
    openSpace: (machineId, workspaceId) => invoke("open_space", { machineId, workspaceId }),
    selectTab: (machineId, tabId) => invoke("select_tab", { machineId, tabId }),
    focusPane: (machineId, paneId) => invoke("focus_pane", { machineId, paneId }),
    sendKey: (machineId, paneId, key) => invoke("send_key", { machineId, paneId, key }),
    sendPaste: (machineId, paneId, text) => invoke("send_paste", { machineId, paneId, text }),
    resizeViewport: (machineId, cols, rows) => invoke("resize_viewport", { machineId, cols, rows }),
    scroll: (machineId, paneId, lines) => invoke("scroll", { machineId, paneId, lines }),
    getMeta: async () => snakeKeys(await invoke("get_meta")) as MetaReplica,
    // INTERFACE.md: set_space_meta takes ONE object {machineId, herdrId, fields, uuid?} and returns the merged Space.
    setSpaceMeta: async (a) => snakeKeys(await invoke("set_space_meta", { machineId: a.machineId, herdrId: a.herdrId, fields: a.fields, uuid: a.uuid })) as SpaceMeta,
    syncNow: async (machineId) => snakeKeys(await invoke("sync_now", { machineId })) as SyncReport,
    syncStatus: async () => snakeKeys(await invoke("sync_status")) as SyncStatus[],
    onMetaChanged: on<MetaChanged>("meta-changed"),
    onSyncStatus: on<SyncStatus>("sync-status"),
    onMachineStatus: on<MachineStatus>("machine-status"),
    onSnapshot: on<Snapshot>("snapshot"),
    onSurface: on<SurfaceEvent>("surface"),
  };
}

let cached: Promise<Api> | null = null;
export function getApi(): Promise<Api> {
  cached ??= inTauri() ? createTauriApi() : Promise.resolve(createMockApi());
  return cached;
}

// ---------------------------------------------------------------- mock backend

type Listeners = {
  status: Set<(s: MachineStatus) => void>;
  snap: Set<(s: Snapshot) => void>;
  surf: Set<(s: SurfaceEvent) => void>;
  meta: Set<(e: MetaChanged) => void>;
  sync: Set<(s: SyncStatus) => void>;
};

const blank = (): Cell => ({ ch: " ", fg: null, bg: null, bold: false, italic: false, underline: false, inverse: false, wide: false });

function writeText(grid: Cell[][], x: number, y: number, text: string, over: Partial<Cell> = {}) {
  if (y < 0 || y >= grid.length) return;
  let cx = x;
  for (const ch of text) {
    const wide = /[　-鿿＀-￯]/.test(ch);
    if (cx >= grid[y].length) break;
    grid[y][cx] = { ...blank(), ...over, ch, wide };
    if (wide && cx + 1 < grid[y].length) grid[y][cx + 1] = { ...blank(), ...over, ch: "" };
    cx += wide ? 2 : 1;
  }
}

export function createMockApi(): Api {
  const L: Listeners = { status: new Set(), snap: new Set(), surf: new Set(), meta: new Set(), sync: new Set() };
  const machines: Machine[] = [
    { id: "local", label: "This PC", kind: "local" },
    { id: "mac", label: "Mac (tailscale)", kind: "ssh", ssh_target: "rishabh@mac", session: "main" },
    { id: "dead", label: "Old laptop", kind: "ssh", ssh_target: "me@old-laptop" },
  ];
  const mkWs = (id: string, label: string, tabs: [string, string[]][], category?: string, presence: string[] = []) => ({
    id, label, category, presence, agent_state: id.endsWith("1") ? "working" : undefined,
    tabs: tabs.map(([tid, panes]) => ({ id: tid, label: tid.replace(/^t-/, ""), panes: panes.map((p) => ({ id: p })) })),
  });
  const snaps: Record<string, Snapshot> = {
    local: { machine_id: "local", workspaces: [
      mkWs("w1", "wooloo-win", [["t-shell", ["p1", "p2"]], ["t-build", ["p3"]]], "claude", ["PC", "Mac"]),
      mkWs("w2", "Study Central", [["t-notes", ["p4"]]], undefined, ["PC"]),
      mkWs("w3", "options-trading", [["t-algo", ["p5"]]]),
      mkWs("w4", "scratch", [["t-main", ["p6"]]]),
    ] },
    mac: { machine_id: "mac", workspaces: [
      mkWs("m1", "Marquee player", [["t-dev", ["q1", "q2"]]], undefined, ["Mac"]),
      mkWs("m2", "Budget digest", [["t-cron", ["q3"]]], "finance"),
    ] },
    dead: { machine_id: "dead", workspaces: [] },
  };
  const status: Record<string, MachineStatus> = {
    local: { machine_id: "local", state: "connected" },
    mac: { machine_id: "mac", state: "connecting", reason: "ssh handshake" },
    dead: { machine_id: "dead", state: "failed", reason: "ssh: connect to host old-laptop port 22: timed out", retry_in_secs: 30 },
  };
  const NODE = "pc-node";
  const NODES: Record<string, string> = { local: NODE, mac: "mac-node", dead: "dead-node" };
  const hostOf = (mid: string) => NODES[mid] ?? mid;
  const uuid = (m: string, h: string) => `mock-${m}-${h}`;
  const metas: SpaceMeta[] = [];
  for (const [mid, sn] of Object.entries(snaps)) sn.workspaces.forEach((w, i) => metas.push({
    uuid: uuid(mid, w.id), host: hostOf(mid), herdr_id: w.id, name: w.label, category: w.category ?? null, color: null, order: i, notes: null, pinned_tabs: [], deleted: false, hlc: {},
  }));
  let lastOk = Date.now() - 2000, syncing = false, lastError: string | undefined;
  const report: SyncReport = { peer: "mac-node", sent: 0, received: 0, stored: 0, rejected: 0, peer_accepted: 0, peer_rejected: 0 };
  const syncRows = (): SyncStatus[] => [{
    machine_id: "mac", peer_label: "Mac (tailscale)", state: lastError ? "offline" : "ok", last_ok_ms: lastOk, pending: 0,
    local_herdr_version: "0.9.2", peer_herdr_version: "0.9.2", syncing, last_error: lastError, last_report: report,
  }];
  const emitSync = () => L.sync.forEach((f) => f(syncRows()[0]));
  const emitMeta = (source: "local" | "sync", mid = "local") => L.meta.forEach((f) => f({ machine_id: mid, source }));
  const typed: Record<string, string> = {};
  const viewport = { cols: 120, rows: 36 };
  const current = new Map<string, string>(); // machine -> tab id
  const scrollOff: Record<string, number> = {};
  let timer: ReturnType<typeof setInterval> | null = null;

  const emitStatus = (s: MachineStatus) => { status[s.machine_id] = s; L.status.forEach((f) => f(s)); };
  const emitSnap = (id: string) => L.snap.forEach((f) => f(snaps[id]));

  function tabPanes(mid: string, tabId: string): string[] {
    for (const w of snaps[mid]?.workspaces ?? []) for (const t of w.tabs) if (t.id === tabId) return t.panes.map((p) => p.id);
    return [];
  }

  function paneGrid(pane: string, cols: number, rows: number, now: Date): PaneSurface["rows_data"] {
    const g = Array.from({ length: rows }, () => Array.from({ length: cols }, blank));
    writeText(g, 0, 0, `herdr mock pane ${pane}`, { bold: true, fg: [137, 180, 250] });
    writeText(g, 0, 1, "bold", { bold: true });
    writeText(g, 5, 1, "italic", { italic: true });
    writeText(g, 12, 1, "underline", { underline: true });
    writeText(g, 22, 1, "inverse", { inverse: true });
    writeText(g, 30, 1, "truecolor", { fg: [250, 179, 135], bg: [49, 50, 68] });
    writeText(g, 40, 1, "日本語ワイド", { fg: [166, 227, 161] });
    for (let i = 0; i < Math.min(cols, 48); i++) {
      const v = Math.round((i / 47) * 255);
      g[2][i] = { ...blank(), ch: " ", bg: [v, 90, 255 - v] };
    }
    writeText(g, 0, 4, `time ${now.toLocaleTimeString()}`, { fg: [249, 226, 175] });
    writeText(g, 0, 6, `$ ${typed[pane] ?? ""}`, { fg: [203, 166, 247] });
    const off = scrollOff[pane] ?? 0;
    if (off) writeText(g, 0, 8, `[scrolled ${off} lines]`, { fg: [243, 139, 168] });
    return g;
  }

  function emitSurface(mid: string, tabId: string) {
    const ids = tabPanes(mid, tabId);
    if (!ids.length) return;
    const now = new Date();
    const half = Math.floor(viewport.cols / 2);
    const panes: PaneSurface[] = ids.map((id, i) => {
      const w = ids.length === 1 ? viewport.cols : i === 0 ? half : viewport.cols - half;
      const x = ids.length === 1 || i === 0 ? 0 : half;
      const rows_data = paneGrid(id, w, viewport.rows, now);
      return {
        pane_id: id, x, y: 0, cols: w, rows: viewport.rows,
        cursor: { x: 2 + (typed[id]?.length ?? 0), y: 6, visible: true, shape: "block" },
        rows_data, dirty_rows: rows_data.map((_, r) => r),
      };
    });
    L.surf.forEach((f) => f({ machine_id: mid, tab_id: tabId, panes }));
  }

  function start() {
    if (timer) return;
    setTimeout(() => { snaps.mac.workspaces.length && emitStatus({ machine_id: "mac", state: "connected" }); }, 1500);
    timer = setInterval(() => { for (const [m, t] of current) if (status[m]?.state === "connected") emitSurface(m, t); }, 1000);
  }

  const reg = <T>(set: Set<(v: T) => void>, cb: (v: T) => void, init?: () => void): Promise<Unlisten> => {
    set.add(cb);
    if (init) setTimeout(init, 0);
    start();
    return Promise.resolve(() => set.delete(cb));
  };

  return {
    listMachines: async () => machines.slice(),
    addMachine: async (m) => {
      const mm: Machine = { id: `m-${Math.random().toString(36).slice(2, 8)}`, label: m.label, kind: "ssh", ssh_target: m.ssh_target, session: m.session };
      machines.push(mm);
      snaps[mm.id] = { machine_id: mm.id, workspaces: [] };
      emitStatus({ machine_id: mm.id, state: "connecting" });
      setTimeout(() => emitStatus({ machine_id: mm.id, state: "failed", reason: "mock: cannot reach host", tried: ["ssh user@host:22"], hint: "check the host is on", retry_in_secs: 20 }), 1200);
      return mm;
    },
    removeMachine: async (id) => { const i = machines.findIndex((m) => m.id === id); if (i >= 0) machines.splice(i, 1); current.delete(id); },
    reconnectMachine: async (id) => {
      emitStatus({ machine_id: id, state: "connecting" });
      setTimeout(() => emitStatus({ machine_id: id, state: id === "dead" ? "failed" : "connected", reason: id === "dead" ? "ssh: timed out" : undefined, retry_in_secs: id === "dead" ? 30 : undefined }), 800);
    },
    startLocalHerdr: async () => {},
    openSpace: async (mid, wid) => {
      const w = snaps[mid]?.workspaces.find((x) => x.id === wid);
      if (!w) return;
      current.set(mid, w.tabs[0].id);
      emitSurface(mid, w.tabs[0].id);
    },
    selectTab: async (mid, tid) => { current.set(mid, tid); emitSurface(mid, tid); },
    focusPane: async () => {},
    sendKey: async (mid, pane, k) => {
      const t = typed[pane] ?? "";
      if (k.text) typed[pane] = t + k.text;
      else if (k.code === "Backspace") typed[pane] = t.slice(0, -1);
      else if (k.code === "Enter") typed[pane] = "";
      const tab = current.get(mid); if (tab) emitSurface(mid, tab);
    },
    sendPaste: async (mid, pane, text) => { typed[pane] = (typed[pane] ?? "") + text.replace(/\r?\n/g, " "); const tab = current.get(mid); if (tab) emitSurface(mid, tab); },
    resizeViewport: async (mid, cols, rows) => { viewport.cols = cols; viewport.rows = rows; const tab = current.get(mid); if (tab) emitSurface(mid, tab); },
    scroll: async (mid, pane, lines) => { scrollOff[pane] = Math.max(0, (scrollOff[pane] ?? 0) + lines); const tab = current.get(mid); if (tab) emitSurface(mid, tab); },
    getMeta: async () => ({ node: NODE, spaces: metas.map((m) => ({ ...m })) }),
    setSpaceMeta: async ({ machineId, herdrId, fields, uuid: u }) => {
      const m = metas.find((x) => (u ? x.uuid === u : x.host === hostOf(machineId) && x.herdr_id === herdrId));
      if (!m) throw new Error("unknown space");
      Object.assign(m, fields);
      const t = Date.now();
      for (const k of Object.keys(fields)) m.hlc[k] = { wall: t, counter: 0 };
      emitMeta("local", machineId);
      return { ...m };
    },
    syncNow: async () => { syncing = false; lastError = undefined; lastOk = Date.now(); emitMeta("sync", "mac"); emitSync(); return { ...report }; },
    syncStatus: async () => syncRows(),
    onMetaChanged: (cb) => reg(L.meta, cb),
    onSyncStatus: (cb) => reg(L.sync, cb),
    onMachineStatus: (cb) => reg(L.status, cb, () => Object.values(status).forEach(cb)),
    onSnapshot: (cb) => reg(L.snap, cb, () => Object.keys(snaps).forEach((id) => { if (status[id].state !== "failed") emitSnap(id); })),
    onSurface: (cb) => reg(L.surf, cb),
  };
}
