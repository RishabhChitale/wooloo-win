import "./style.css";
import { getApi, type Api } from "./api";
import { categoryLineColors } from "./theme";
import { CATEGORY_TITLES, groupSpaces } from "./grouping";
import { clipboardShortcut, mapKeyEvent } from "./keys";
import { measureCell, PaneView } from "./terminal";
import { applyMeta, hostOf, peerNodes, moveOrder, presenceLabel, syncBadge, validName, validNotes, isHexColor, PALETTE, type DisplaySpace } from "./meta";
import { CATEGORY_ORDER } from "./grouping";
import { diagnosticsText, failureView, type FailureView } from "./diagnostics";
import { skewBanners } from "./version-skew";
import type { SpaceMetaPatch, Machine, MachineStatus, Snapshot, SpaceMeta, SurfaceEvent, SyncStatus, DiagReport } from "./types";

declare const __APP_VERSION__: string;
const APP_VERSION = typeof __APP_VERSION__ === "string" ? __APP_VERSION__ : "dev";
const app = document.getElementById("app")!;
app.innerHTML = `
  <aside id="rail"><div class="rail-head"><span>MACHINES</span><span class="rail-btns"><button id="pair" title="Pair with another computer running wooloo">Pair device</button><button id="add" title="Add machine">+ Add</button></span></div><div id="machines"></div></aside>
  <main id="main"><div id="skew" hidden></div><div id="tabs"></div><div id="work"><div id="panes" tabindex="0"><div id="placeholder">Select a Space</div></div><aside id="notes" hidden></aside></div></main>
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
let metas: SpaceMeta[] = [];
let metaNode = "";
let syncRows: SyncStatus[] = [];
let notesOpen = false;
let machines: Machine[] = [];
const status = new Map<string, MachineStatus>();
const snaps = new Map<string, Snapshot>();
const surfaces = new Map<string, SurfaceEvent>(); // key machine|tab
let selMachine: string | null = null, selSpace: string | null = null, selTab: string | null = null;
let focusedPane: string | null = null;
const views = new Map<string, PaneView>();

const fire = (p: Promise<unknown>) => { p.catch((e) => console.warn("backend call failed", e)); }; // never block on dead machines
const skey = (m: string, t: string) => `${m}|${t}`;

function failureBox(m: Machine, st: MachineStatus | undefined, fv: FailureView): HTMLElement {
  const box = document.createElement("div"); box.className = "fail";
  const t = document.createElement("div"); t.className = "fail-reason"; t.textContent = fv.reason; box.appendChild(t);
  if (fv.hint) { const h = document.createElement("div"); h.className = "fail-hint"; h.textContent = fv.hint; box.appendChild(h); }
  if (fv.tried.length) {
    const tr = document.createElement("div"); tr.className = "fail-tried";
    tr.textContent = "Tried:\n" + fv.tried.map((x) => `- ${x}`).join("\n"); box.appendChild(tr);
  }
  if (fv.retry) { const r = document.createElement("div"); r.className = "fail-hint"; r.textContent = fv.retry; box.appendChild(r); }
  if (fv.sessions.length) {
    const sb = document.createElement("div"); sb.className = "fail-btns";
    const lab = document.createElement("span"); lab.className = "fail-hint"; lab.textContent = "Use session:"; sb.appendChild(lab);
    for (const name of fv.sessions) {
      const b = document.createElement("button"); b.textContent = name;
      b.addEventListener("click", () => fire(api.setMachineSession(m.id, name).then((nm) => { const i = machines.findIndex((x) => x.id === m.id); if (i >= 0) machines[i] = nm; renderRail(); })));
      sb.appendChild(b);
    }
    box.appendChild(sb);
  }
  const bar = document.createElement("div"); bar.className = "fail-btns";
  if (fv.canStart) {
    const b = document.createElement("button"); b.textContent = "Start Herdr"; b.title = "Start the local Herdr server";
    b.addEventListener("click", () => fire(api.startLocalHerdr(m.id))); bar.appendChild(b);
  }
  const c = document.createElement("button"); c.textContent = "Copy diagnostics";
  c.addEventListener("click", async () => {
    const text = diagnosticsText(m, st, { appVersion: APP_VERSION, userAgent: navigator.userAgent });
    try { await navigator.clipboard.writeText(text); c.textContent = "Copied"; } catch { c.textContent = "Copy failed"; }
    setTimeout(() => { c.textContent = "Copy diagnostics"; }, 1500);
  });
  bar.appendChild(c); box.appendChild(bar);
  return box;
}

const connTests = new Map<string, DiagReport | "running" | string>();
function runConnTest(id: string) {
  connTests.set(id, "running"); renderRail();
  api.testConnection(id).then((r) => connTests.set(id, r), (e) => connTests.set(id, String(e))).then(() => renderRail());
}
function connTestBox(id: string, ct: DiagReport | string): HTMLElement {
  const box = document.createElement("div"); box.className = "fail conn-test";
  if (typeof ct === "string") {
    const l = document.createElement("div"); l.textContent = ct === "running" ? "Testing connection…" : `Test failed to run: ${ct}`; box.appendChild(l);
  } else {
    for (const s of ct.steps) {
      const l = document.createElement("div"); l.className = `conn-step ${s.status}`;
      l.textContent = `${s.status === "ok" ? "ok" : s.status === "skip" ? "skip" : s.status === "warn" ? "WARN" : "FAIL"}  ${s.label}${s.detail ? ` - ${s.detail}` : ""}`; box.appendChild(l);
    }
    if (ct.hint) { const h = document.createElement("div"); h.className = "conn-hint"; h.textContent = ct.hint; box.appendChild(h); }
  }
  if (ct !== "running") {
    const b = document.createElement("button"); b.textContent = "Close"; b.addEventListener("click", () => { connTests.delete(id); renderRail(); }); box.appendChild(b);
  }
  return box;
}

function renderRail() {
  machinesEl.replaceChildren();
  for (const m of machines) {
    const st = status.get(m.id);
    const row = document.createElement("div");
    row.className = "machine-row";
    const tip = st ? `${st.state}${st.reason ? `: ${st.reason}` : ""}${st.retry_in_secs ? ` (retry in ${st.retry_in_secs}s)` : ""}` : "unknown";
    row.innerHTML = `<span class="dot ${st?.state ?? ""}" title="${esc(tip)}"></span><span class="label" title="${esc(m.ssh_target ?? "local")}">${esc(m.label)}</span>
      <span class="acts"><button data-a="pair" title="Pair device">&#128279;</button>${m.kind === "local" ? "" : '<button data-a="tc" title="Test connection">&#9889;</button>'}<button data-a="re" title="Reconnect">&#8635;</button><button data-a="rm" title="Remove machine">&#10005;</button></span>`;
    row.querySelector('[data-a="pair"]')!.addEventListener("click", () => openPair?.());
    row.querySelector('[data-a="tc"]')?.addEventListener("click", () => runConnTest(m.id));
    row.querySelector('[data-a="re"]')!.addEventListener("click", () => fire(api.reconnectMachine(m.id)));
    row.querySelector('[data-a="rm"]')!.addEventListener("click", () => {
      fire(api.removeMachine(m.id));
      machines = machines.filter((x) => x.id !== m.id); snaps.delete(m.id); status.delete(m.id);
      if (selMachine === m.id) { selMachine = selSpace = selTab = null; renderMain(); }
      renderRail();
    });
    machinesEl.appendChild(row);
    const ct = connTests.get(m.id);
    if (ct) machinesEl.appendChild(connTestBox(m.id, ct));
    const fv = failureView(m, st);
    if (fv) machinesEl.appendChild(failureBox(m, st, fv));
    const sy = syncRows.find((r) => r.machine_id === m.id);
    if (sy) {
      const b = document.createElement("div"); b.className = `sync-badge ${sy.syncing ? "syncing" : sy.state === "ok" ? "synced" : "offline"}`;
      b.textContent = syncBadge(sy, m.label, Date.now()); b.title = "Click to sync now";
      b.addEventListener("click", () => syncNow(m.id));
      machinesEl.appendChild(b);
    }
    const spaces = applyMeta(hostOf(m, metaNode, peerNodes(syncRows)), snaps.get(m.id)?.workspaces ?? [], metas);
    if (!spaces.length) {
      const e = document.createElement("div"); e.className = "empty";
      e.textContent = st?.state === "failed" ? "unreachable (details above)" : st?.state === "connecting" ? "connecting…" : "no spaces";
      machinesEl.appendChild(e);
    }
    for (const g of groupSpaces(spaces)) {
      const h = document.createElement("div"); h.className = "cat"; h.textContent = CATEGORY_TITLES[g.category] ?? g.title;
      const line = document.createElement("div"); line.className = "cat-line"; line.style.background = categoryLineColors[g.category];
      machinesEl.append(h, line);
      for (const w of g.spaces) {
        const s = document.createElement("div");
        s.className = "space" + (selMachine === m.id && selSpace === w.id ? " active" : "");
        s.draggable = !!w.uuid;
        if (w.color) s.style.borderLeft = `3px solid ${w.color}`;
        const pres = presenceLabel(w.presence);
        s.innerHTML = `<span class="name">${esc(w.label)}</span>${pres ? `<span class="presence" title="${esc(pres)}">${esc(pres)}</span>` : ""}${w.agent_state ? `<span class="agent" title="agent ${esc(w.agent_state)}">&#9679;</span>` : ""}`;
        s.addEventListener("click", () => {
          selMachine = m.id; selSpace = w.id; selTab = w.tabs[0]?.id ?? null; focusedPane = null;
          fire(api.openSpace(m.id, w.id));
          renderRail(); renderMain(); reportViewport();
        });
        wireSpaceRow(s, m.id, w, g.spaces);
        machinesEl.appendChild(s);
      }
    }
  }
}

function currentSpace(): DisplaySpace | undefined {
  const sm = machines.find((x) => x.id === selMachine);
  return sm ? applyMeta(hostOf(sm, metaNode, peerNodes(syncRows)), snaps.get(sm.id)?.workspaces ?? [], metas).find((w) => w.id === selSpace) : undefined;
}

const patch = (mid: string, w: DisplaySpace, p: SpaceMetaPatch) => {
  // A Space without a mapped uuid has no metadata yet: never write (protocol 4.1).
  if (!w.uuid) return;
  const m = machines.find((x) => x.id === mid);
  if (!m) return;
  fire(api.setSpaceMeta({ machineId: mid, herdrId: w.id, fields: p, uuid: w.uuid }).then((sp) => {
    metas = metas.some((x) => x.uuid === sp.uuid) ? metas.map((x) => (x.uuid === sp.uuid ? sp : x)) : [...metas, sp];
    renderRail(); renderNotes(); if (selSpace) renderMain();
  }));
};

/** Rename (double-click), colour/category menu (context menu) and drag reorder within the category group. */
function wireSpaceRow(el: HTMLElement, mid: string, w: DisplaySpace, group: DisplaySpace[]) {
  el.addEventListener("dblclick", (e) => {
    e.preventDefault();
    if (!w.uuid) return;
    const name = el.querySelector(".name") as HTMLElement;
    const inp = document.createElement("input"); inp.className = "rename"; inp.value = w.label; inp.maxLength = 200;
    name.replaceWith(inp); inp.focus(); inp.select();
    let done = false;
    const finish = (save: boolean) => {
      if (done) return; done = true;
      const v = inp.value.trim();
      if (save && v !== w.label && validName(v)) patch(mid, w, { name: v });
      renderRail();
    };
    inp.addEventListener("keydown", (k) => { k.stopPropagation(); if (k.key === "Enter") finish(true); else if (k.key === "Escape") finish(false); });
    inp.addEventListener("blur", () => finish(true));
    inp.addEventListener("click", (c) => c.stopPropagation());
  });
  el.addEventListener("contextmenu", (e) => { e.preventDefault(); if (w.uuid) openMenu(e.clientX, e.clientY, mid, w); });
  el.addEventListener("dragstart", (e) => { e.dataTransfer?.setData("text/plain", w.id); e.dataTransfer!.effectAllowed = "move"; el.classList.add("dragging"); });
  el.addEventListener("dragend", () => el.classList.remove("dragging"));
  el.addEventListener("dragover", (e) => { e.preventDefault(); el.classList.add("drop"); });
  el.addEventListener("dragleave", () => el.classList.remove("drop"));
  el.addEventListener("drop", (e) => {
    e.preventDefault(); el.classList.remove("drop");
    const from = group.findIndex((x) => x.id === e.dataTransfer?.getData("text/plain"));
    const to = group.findIndex((x) => x.id === w.id);
    if (from < 0 || to < 0 || from === to) return;
    patch(mid, group[from], { order: moveOrder(group.map((x) => x.order), from, to) });
  });
}

const menuEl = document.createElement("div"); menuEl.id = "menu"; menuEl.hidden = true; document.body.appendChild(menuEl);
document.addEventListener("click", () => { menuEl.hidden = true; });
function openMenu(x: number, y: number, mid: string, w: DisplaySpace) {
  menuEl.replaceChildren();
  const sw = document.createElement("div"); sw.className = "swatches";
  for (const c of [null, ...PALETTE]) {
    const b = document.createElement("button"); b.className = "swatch"; b.title = c ?? "no colour";
    b.style.background = c ?? "transparent"; if (!c) b.textContent = "\u2715";
    b.addEventListener("click", () => { if (c === null || isHexColor(c)) patch(mid, w, { color: c }); });
    sw.appendChild(b);
  }
  const cats = document.createElement("div"); cats.className = "cats";
  for (const c of [null, ...CATEGORY_ORDER]) {
    const b = document.createElement("button"); b.textContent = c === null ? "Automatic" : CATEGORY_TITLES[c];
    b.addEventListener("click", () => patch(mid, w, { category: c }));
    cats.appendChild(b);
  }
  menuEl.append(sw, cats);
  menuEl.style.left = `${Math.min(x, window.innerWidth - 200)}px`; menuEl.style.top = `${Math.min(y, window.innerHeight - 260)}px`;
  menuEl.hidden = false;
}

const skewEl = document.getElementById("skew")!;
function renderSkew() {
  const banners = skewBanners(syncRows);
  skewEl.replaceChildren(...banners.map((b) => {
    const d = document.createElement("div");
    d.className = b.severity;
    d.textContent = b.message;
    return d;
  }));
  skewEl.hidden = banners.length === 0;
}

async function refreshMeta() {
  try { const r = await api.getMeta(); metaNode = r.node; metas = r.spaces; } catch (e) { console.warn("get_meta failed", e); }
  try { syncRows = await api.syncStatus(); } catch (e) { console.warn("sync_status failed", e); }
  renderRail(); renderNotes(); renderSkew(); if (selSpace) renderMain();
}
async function syncNow(machineId: string) {
  syncRows = syncRows.map((r) => (r.machine_id === machineId ? { ...r, syncing: true } : r)); renderRail();
  try { await api.syncNow(machineId); } catch (e) { console.warn("sync_now failed", e); }
  await refreshMeta();
}

const notesEl = document.getElementById("notes")!;
let notesFor = "";
function renderNotes() {
  const w = currentSpace();
  notesEl.hidden = !(notesOpen && w);
  if (notesEl.hidden || !w) return;
  const id = `${selMachine}|${w.id}`;
  const ta = notesEl.querySelector("textarea") as HTMLTextAreaElement | null;
  if (ta && notesFor === id) { if (document.activeElement !== ta) ta.value = w.notes ?? ""; return; }
  notesFor = id;
  notesEl.innerHTML = `<div class="notes-head">Notes <span id="notes-info"></span></div><textarea placeholder="Notes sync to your other machine. Do not paste secrets here."${w.uuid ? "" : " disabled"}></textarea>`;
  const t = notesEl.querySelector("textarea")!; t.value = w.notes ?? "";
  const info = notesEl.querySelector("#notes-info") as HTMLElement;
  let timer: ReturnType<typeof setTimeout> | undefined;
  t.addEventListener("input", () => {
    const ok = validNotes(t.value); info.textContent = ok ? "" : "too long or has control characters"; t.classList.toggle("bad", !ok);
    clearTimeout(timer);
    if (ok) timer = setTimeout(() => { const cur = currentSpace(); if (cur && `${selMachine}|${cur.id}` === id) patch(selMachine!, cur, { notes: t.value === "" ? null : t.value }); }, 600);
  });
}

function renderMain() {
  tabsEl.replaceChildren();
  const ws = currentSpace();
  for (const t of ws?.tabs ?? []) {
    const el = document.createElement("div");
    el.className = "tab" + (t.id === selTab ? " active" : ""); el.textContent = t.label;
    el.addEventListener("click", () => { selTab = t.id; fire(api.selectTab(selMachine!, t.id)); renderMain(); });
    tabsEl.appendChild(el);
  }
  if (ws) {
    const nb = document.createElement("button"); nb.id = "notes-btn"; nb.textContent = notesOpen ? "Hide notes" : "Notes";
    nb.addEventListener("click", () => { notesOpen = !notesOpen; renderMain(); reportViewport(); });
    tabsEl.appendChild(nb);
  }
  renderNotes();
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
let openPair: (() => void) | null = null;
import("./pair").then((m) => { openPair = m.initPair(dlg, (m2, replaced) => { for (const id of replaced) { const i = machines.findIndex((x) => x.id === id); if (i >= 0 && id !== m2.id) machines.splice(i, 1); } if (!machines.some((x) => x.id === m2.id)) machines.push(m2); renderRail(); }); });
$("pair").addEventListener("click", () => openPair?.());
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
  await api.onMetaChanged(() => { void refreshMeta(); });
  await api.onSyncStatus((st) => {
    syncRows = syncRows.some((r) => r.machine_id === st.machine_id) ? syncRows.map((r) => (r.machine_id === st.machine_id ? st : r)) : [...syncRows, st];
    renderRail(); renderSkew();
  });
  machines = await api.listMachines();
  void refreshMeta();
  setInterval(renderRail, 5000); // refresh the "2s ago" age text
  renderRail();
})();
