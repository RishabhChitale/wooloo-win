// "Pair device" dialog: peer-to-peer pairing with another computer running wooloo (Mac or Windows).
// Self-contained: builds its own <dialog> and adds one button to the add-machine dialog.
import type { Machine } from "./types";

interface LocalInfo { code: string; peer_id: string; public_fp: string; host_fp: string }
interface Review { label: string; target: string; session: string; herdr_version: string; peer_id: string; public_fp: string; host_fp: string; admin: boolean; authorized_keys_path: string }
interface Accepted { machine: Machine; peer_id: string; needs_elevation: string | null; authorized_keys_path: string }

async function call<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown };
  if (!w.__TAURI_INTERNALS__) throw new Error("Pairing needs the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

const esc = (s: string) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

export function initPair(addDlg: HTMLDialogElement, onAdded: (m: Machine) => void): void {
  const dlg = document.createElement("dialog");
  dlg.id = "pairdlg";
  dlg.innerHTML = `
    <div style="min-width:420px;max-width:560px;display:grid;gap:8px">
      <h3 style="margin:0">Pair device</h3>
      <div><b>1. Show this PC's code</b></div>
      <div style="display:flex;gap:6px"><input id="pair-target" placeholder="this PC as the peer reaches it: user@100.x.y.z" style="flex:1"><button id="pair-make" type="button">Make code</button></div>
      <div id="pair-local" style="font-size:12px"></div>
      <div><b>2. Paste the other device's code</b></div>
      <textarea id="pair-code" rows="3" placeholder="wooloo-pair-v1:..." style="width:100%"></textarea>
      <button id="pair-review" type="button">Review</button>
      <div id="pair-info" style="font-size:12px"></div>
      <label id="pair-inc" style="display:none;font-size:12px"><input id="pair-incbox" type="checkbox"> Add an Include line to ~/.ssh/config so plain ssh also knows this peer</label>
      <div style="display:flex;gap:6px;justify-content:flex-end"><button id="pair-accept" type="button" disabled>Accept pairing</button><button id="pair-close" type="button">Close</button></div>
    </div>`;
  document.body.appendChild(dlg);
  const q = <T extends HTMLElement>(id: string) => dlg.querySelector<T>("#" + id)!;
  const local = q("pair-local"), info = q("pair-info"), accept = q<HTMLButtonElement>("pair-accept");
  const fail = (el: HTMLElement, e: unknown) => { el.innerHTML = `<span style="color:#c33">${esc(String(e))}</span>`; };

  q("pair-make").addEventListener("click", async () => {
    try {
      const r = await call<LocalInfo>("pair_local_info", { target: q<HTMLInputElement>("pair-target").value });
      local.innerHTML = `Send this code to the other device (and compare fingerprints there):<br><textarea readonly rows="3" style="width:100%">${esc(r.code)}</textarea>key ${esc(r.public_fp)}<br>host ${esc(r.host_fp)}`;
    } catch (e) { fail(local, e); }
  });

  let reviewed = "";
  q("pair-review").addEventListener("click", async () => {
    accept.disabled = true;
    const code = q<HTMLTextAreaElement>("pair-code").value.trim();
    try {
      const r = await call<Review>("pair_review", { code });
      reviewed = code;
      info.innerHTML = `<b>${esc(r.label)}</b> (id ${esc(r.peer_id)}) at <code>${esc(r.target)}</code>, session ${esc(r.session)}, herdr ${esc(r.herdr_version)}<br>
        key ${esc(r.public_fp)}<br>host ${esc(r.host_fp)}<br>
        Accepting lets that device's key run only Herdr bridge commands on this PC (entry in <code>${esc(r.authorized_keys_path)}</code>). Check the fingerprints match what the other device shows.`;
      q("pair-inc").style.display = "block";
      accept.disabled = false;
    } catch (e) { fail(info, e); }
  });

  accept.addEventListener("click", async () => {
    accept.disabled = true;
    try {
      const r = await call<Accepted>("pair_accept", { code: reviewed, include_ssh_config: q<HTMLInputElement>("pair-incbox").checked });
      onAdded(r.machine);
      info.innerHTML = r.needs_elevation
        ? `Paired, but <code>${esc(r.authorized_keys_path)}</code> needs an elevated write. Run PowerShell as Administrator and append this line (see docs/windows-openssh-setup.md):<br><textarea readonly rows="4" style="width:100%">${esc(r.needs_elevation)}</textarea>`
        : `Paired with ${esc(r.machine.label)}.`;
    } catch (e) { fail(info, e); }
  });

  q("pair-close").addEventListener("click", () => dlg.close());

  const btn = document.createElement("button");
  btn.type = "button";
  btn.textContent = "Pair device…";
  btn.addEventListener("click", () => { addDlg.close(); dlg.showModal(); });
  (addDlg.querySelector("form") ?? addDlg).appendChild(btn);
}
