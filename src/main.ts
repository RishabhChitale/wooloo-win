import "./style.css";
import { getApi, type Api } from "./api";
import { categoryLineColors } from "./theme";
import { CATEGORY_TITLES, groupSpaces } from "./grouping";
import { clipboardShortcut, mapKeyEvent } from "./keys";
import { measureCell, PaneView } from "./terminal";
import type { Machine, MachineStatus, Snapshot, SurfaceEvent } from "./types";

const app = document.getElementById("app")!;
app.innerHTML = `
  <aside id="rail"><div class="rail-head"><span>MACHINES</span><button id="add" title="Add machine">+ Add</button></div><div id="machines"></div></aside>
  <main id="main"><div id="tabs"></div><div id="panes" tabindex="0"><div id="placeholder">Select a Space</div></div></main>
  <dialog id="dlg"><form method="dialog" id="addform">
    <strong>Add machine</strong>
    <label>Label</label><input name="label" required placeholder="Work PC">
    <label>SSH target</label><input name="ssh" required placeholder="user@host">
    <label>Herdr session (optional)</label><input name="session" placeholder="main">
    <div class="row"><button type="button" id="cancel">Cancel</button><button class="primary" value="ok">Add</button></div>
  </form></dialog>`;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const machinesEl = $("machines"), tabsEl = $("tabs"), panesEl = $("panes"), dlg = $<HTMLDialogElement>("dlg");

let api: Api;
let machines: Machine[] = [];
const status = new Map<string, MachineStatus>();
const snaps = new Map<string, Snapshot>();
const surfaces = new Map<string, SurfaceEvent>(); // key machine|tab
let selMachine: string | null = null, selSpace: string | null = null, selTab: string | null = null;
let focusedPane: string | null = null;
const views = new Map<string, PaneView>();

const fire = (p: Promise<unknown>) => { p.catch((e) => console.warn("backend call failed", e)); }; // never block on dead machines
const skey = (m: string, t: string) => `${m}|${t}`;

function renderRail() {
  machinesEl.replaceChildren();
  for (const m of machines) {
    const st = status.get(m.id);
    const row = document.createElement("div");
    row.className = "machine-row";
    const tip = st ? `${st.state}${st.reason ? `: ${st.reason}` : ""}${st.retry_in_secs ? ` (retry in ${st.retry_in_secs}s)` : ""}` : "unknown";
    row.innerHTML = `<span class="dot ${st?.state ?? ""}" title="${esc(tip)}"></span><span class="label" title="${esc(m.ssh_target ?? "local")}">${esc(m.label)}</span>
      <span class="acts"><button data-a="re" title="Reconnect">&#8635;</button><button data-a="rm" title="Remove machine">&#10005;</button></span>`;
    row.querySelector('[data-a="re"]')!.addEventListener("click", () => fire(api.reconnectMachine(m.id)));
    row.querySelector('[data-a="rm"]')!.addEventListener("click", () => {
      fire(api.removeMachine(m.id));
      machines = machines.filter((x) => x.id !== m.id); snaps.delete(m.id); status.delete(m.id);
      if (selMachine === m.id) { selMachine = selSpace = selTab = null; renderMain(); }
      renderRail();
    });
    machinesEl.appendChild(row);
    const spaces = snaps.get(m.id)?.workspaces ?? [];
    if (!spaces.length) {
      const e = document.createElement("div"); e.className = "empty";
      e.textContent = st?.state === "failed" ? "unreachable" : st?.state === "connecting" ? "connecting…" : "no spaces";
      machinesEl.appendChild(e);
    }
    for (const g of groupSpaces(spaces)) {
      const h = document.createElement("div"); h.className = "cat"; h.textContent = CATEGORY_TITLES[g.category] ?? g.title;
      const line = document.createElement("div"); line.className = "cat-line"; line.style.background = categoryLineColors[g.category];
      machinesEl.append(h, line);
      for (const w of g.spaces) {
        const s = document.createElement("div");
        s.className = "space" + (selMachine === m.id && selSpace === w.id ? " active" : "");
        s.innerHTML = `<span>${esc(w.label)}</span>${w.agent_state ? `<span class="agent" title="agent ${esc(w.agent_state)}">&#9679;</span>` : ""}`;
        s.addEventListener("click", () => {
          selMachine = m.id; selSpace = w.id; selTab = w.tabs[0]?.id ?? null; focusedPane = null;
          fire(api.openSpace(m.id, w.id));
          renderRail(); renderMain(); reportViewport();
        });
        machinesEl.appendChild(s);
      }
    }
  }
}

function currentSpace() { return snaps.get(selMachine ?? "")?.workspaces.find((w) => w.id === selSpace); }

function renderMain() {
  tabsEl.replaceChildren();
  const ws = currentSpace();
  for (const t of ws?.tabs ?? []) {
    const el = document.createElement("div");
    el.className = "tab" + (t.id === selTab ? " active" : ""); el.textContent = t.label;
    el.addEventListener("click", () => { selTab = t.id; fire(api.selectTab(selMachine!, t.id)); renderMain(); });
    tabsEl.appendChild(el);
  }
  renderPanes();
}

function renderPanes() {
  const ev = selMachine && selTab ? surfaces.get(skey(selMachine, selTab)) : undefined;
  const ph = $("placeholder");
  ph.style.display = ev ? "none" : "grid";
  ph.textContent = selSpace ? "Waiting for terminal…" : "Select a Space";
  const live = new Set<string>();
  if (ev) {
    const totalCols = Math.max(1, ...ev.panes.map((p) => p.x + p.cols));
    const totalRows = Math.max(1, ...ev.panes.map((p) => p.y + p.rows));
    if (!focusedPane || !ev.panes.some((p) => p.pane_id === focusedPane)) focusedPane = ev.panes[0]?.pane_id ?? null;
    for (const p of ev.panes) {
      live.add(p.pane_id);
      let v = views.get(p.pane_id);
      if (!v) {
        const mid = ev.machine_id, pid = p.pane_id;
        v = new PaneView(pid, {
          focus: () => { setFocus(pid); fire(api.focusPane(mid, pid)); },
          paste: (t) => fire(api.sendPaste(mid, pid, t)),
          scroll: (n) => fire(api.scroll(mid, pid, n)),
        });
        views.set(pid, v);
      }
      if (v.el.parentElement !== panesEl) panesEl.appendChild(v.el);
      v.setFocused(p.pane_id === focusedPane);
      v.update(p, totalCols, totalRows);
    }
  }
  for (const [id, v] of views) if (!live.has(id)) { v.destroy(); views.delete(id); }
}

function setFocus(pid: string) {
  focusedPane = pid;
  views.forEach((v, id) => v.setFocused(id === pid));
  panesEl.focus();
}

async function copySelection() {
  const v = focusedPane ? views.get(focusedPane) : undefined;
  const t = v?.selectedText();
  if (t) { try { await navigator.clipboard.writeText(t); } catch (e) { console.warn("clipboard write failed", e); } }
}
async function pasteClipboard() {
  if (!selMachine || !focusedPane) return;
  try { const t = await navigator.clipboard.readText(); if (t) fire(api.sendPaste(selMachine, focusedPane, t)); } catch (e) { console.warn("clipboard read failed", e); }
}

panesEl.addEventListener("keydown", (e) => {
  const cs = clipboardShortcut(e);
  if (cs) { e.preventDefault(); void (cs === "copy" ? copySelection() : pasteClipboard()); return; }
  if (!selMachine || !focusedPane) return;
  const k = mapKeyEvent(e);
  if (!k) return;
  e.preventDefault();
  fire(api.sendKey(selMachine, focusedPane, k));
});

let lastVp = "";
function reportViewport() {
  if (!selMachine) return;
  const { w, h } = measureCell();
  const cols = Math.max(2, Math.floor(panesEl.clientWidth / w)), rows = Math.max(2, Math.floor(panesEl.clientHeight / h));
  const key = `${selMachine}:${cols}x${rows}`;
  if (key === lastVp) return;
  lastVp = key;
  fire(api.resizeViewport(selMachine, cols, rows));
}
new ResizeObserver(() => reportViewport()).observe(panesEl);

$("add").addEventListener("click", () => dlg.showModal());
import("./pair").then((m) => m.initPair(dlg, (m2) => { if (!machines.some((x) => x.id === m2.id)) machines.push(m2); renderRail(); }));
$("cancel").addEventListener("click", () => dlg.close());
$<HTMLFormElement>("addform").addEventListener("submit", (e) => {
  const f = e.target as HTMLFormElement;
  const d = new FormData(f);
  const label = String(d.get("label") ?? "").trim(), ssh = String(d.get("ssh") ?? "").trim(), session = String(d.get("session") ?? "").trim();
  if (!label || !ssh) return;
  api.addMachine({ label, ssh_target: ssh, ...(session ? { session } : {}) })
    .then((m) => { if (!machines.some((x) => x.id === m.id)) machines.push(m); renderRail(); })
    .catch((err) => console.warn("add_machine failed", err));
  f.reset();
});

function esc(s: string) { return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!); }

(async () => {
  api = await getApi();
  await api.onMachineStatus((s) => { status.set(s.machine_id, s); renderRail(); });
  await api.onSnapshot((s) => { snaps.set(s.machine_id, s); renderRail(); if (s.machine_id === selMachine) renderMain(); });
  await api.onSurface((ev) => {
    surfaces.set(skey(ev.machine_id, ev.tab_id), ev);
    if (ev.machine_id === selMachine && ev.tab_id === selTab) renderPanes();
  });
  machines = await api.listMachines();
  renderRail();
})();
