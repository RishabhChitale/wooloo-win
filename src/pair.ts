// "Pair device" dialog: peer-to-peer pairing with another computer running wooloo (Mac or Windows).
// Self-contained: builds its own <dialog> and adds one button to the add-machine dialog.
import type { Machine } from "./types";

interface Defaults { target: string; user: string; host: string; notes: string[] }
interface LocalInfo { code: string; peer_id: string; public_fp: string; host_fp: string; herdr_path?: string | null }
interface Review { label: string; target: string; session: string; herdr_version: string; peer_id: string; public_fp: string; host_fp: string; admin: boolean; authorized_keys_path: string; herdr_path?: string | null }
interface Accepted { machine: Machine; peer_id: string; needs_elevation: string | null; elevated_commands: string | null; elevation_reason: string | null; authorized_keys_path: string; replaced?: string[] }

async function call<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown };
  if (!w.__TAURI_INTERNALS__) throw new Error("Pairing needs the desktop app.");
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

const esc = (s: string) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

export function initPair(addDlg: HTMLDialogElement, onAdded: (m: Machine, replaced: string[]) => void): () => void {
  const dlg = document.createElement("dialog");
  dlg.id = "pairdlg";
  dlg.innerHTML = `
    <div style="min-width:420px;max-width:560px;display:grid;gap:8px">
      <h3 style="margin:0">Pair device</h3>
      <div><b>1. Show this PC's code</b></div>
      <div style="display:flex;gap:6px"><input id="pair-target" placeholder="this PC as the peer reaches it: user@100.x.y.z" style="flex:1"><button id="pair-make" type="button">Generate code</button></div>
      <div id="pair-notes" style="font-size:12px;color:#c80"></div>
      <div id="pair-local" style="font-size:12px"></div>
      <div><b>2. Enter a code</b></div>
      <textarea id="pair-code" rows="3" placeholder="wooloo-pair-v1:..." style="width:100%"></textarea>
      <button id="pair-review" type="button">Review</button>
      <div id="pair-info" style="font-size:12px"></div>
      <label id="pair-inc" style="display:none;font-size:12px"><input id="pair-incbox" type="checkbox" checked> Add an Include line to ~/.ssh/config so plain ssh also knows this peer</label>
      <div style="display:flex;gap:6px;justify-content:flex-end"><button id="pair-accept" type="button" disabled>Accept pairing</button><button id="pair-close" type="button">Close</button></div>
    </div>`;
  document.body.appendChild(dlg);
  const q = <T extends HTMLElement>(id: string) => dlg.querySelector<T>("#" + id)!;
  const local = q("pair-local"), info = q("pair-info"), accept = q<HTMLButtonElement>("pair-accept");
  const fail = (el: HTMLElement, e: unknown) => { el.innerHTML = `<span style="color:#c33">${esc(String(e))}</span>`; };

  const prefill = async () => {
    try {
      const d = await call<Defaults>("pair_defaults", {});
      const box = q<HTMLInputElement>("pair-target");
      if (!box.value) box.value = d.target;
      q("pair-notes").innerHTML = d.notes.map((n) => esc(n)).join("<br>");
    } catch (e) { q("pair-notes").textContent = String(e); }
  };

  q("pair-make").addEventListener("click", async () => {
    try {
      const r = await call<LocalInfo>("pair_local_info", { target: q<HTMLInputElement>("pair-target").value });
      local.innerHTML = `Send this code to the other device (and compare fingerprints there):<br><textarea readonly rows="3" style="width:100%">${esc(r.code)}</textarea>key ${esc(r.public_fp)}<br>host ${esc(r.host_fp)}<br>Herdr: ${esc(r.herdr_path ?? "not found")}`;
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
        key ${esc(r.public_fp)}<br>host ${esc(r.host_fp)}<br>Herdr: ${esc(r.herdr_path ?? "not found")}<br>
        Accepting lets that device's key run only Herdr bridge commands on this PC (entry in <code>${esc(r.authorized_keys_path)}</code>). Check the fingerprints match what the other device shows.`;
      q("pair-inc").style.display = "block";
      accept.disabled = false;
    } catch (e) { fail(info, e); }
  });

  accept.addEventListener("click", async () => {
    accept.disabled = true;
    try {
      const r = await call<Accepted>("pair_accept", { code: reviewed, include_ssh_config: q<HTMLInputElement>("pair-incbox").checked });
      onAdded(r.machine, r.replaced ?? []);
      info.innerHTML = r.needs_elevation
        ? `<div style="border:1px solid #c80;border-radius:6px;padding:8px;display:grid;gap:6px">
          <b>One more step: allow the other device to log in to this PC</b>
          <div>Windows keeps administrator keys in <code>${esc(r.authorized_keys_path)}</code>, which only an elevated (Administrator) process can write (${esc(r.elevation_reason ?? "needs an elevated write")}).</div>
          <ol style="margin:0;padding-left:18px"><li>Click <b>Copy commands</b>.</li><li>Open the Start menu, type PowerShell, right-click Windows PowerShell and choose <b>Run as administrator</b>.</li><li>Paste and press Enter.</li></ol>
          <div>What it does: adds one key line, limited to Herdr bridge commands, to that file (skipped if already there) and sets the file permissions sshd requires. Nothing else changes.</div>
          <textarea id="pair-elev" readonly rows="8" style="width:100%;font-family:monospace">${esc(r.elevated_commands ?? "")}</textarea>
          <div><button id="pair-copy" type="button">Copy commands</button> <span id="pair-copied" style="color:#2a7"></span></div>
        </div>`
        : `Paired with ${esc(r.machine.label)}.`;
      info.querySelector("#pair-copy")?.addEventListener("click", () => { void navigator.clipboard?.writeText(r.elevated_commands ?? "").then(() => { const c = info.querySelector("#pair-copied"); if (c) c.textContent = "Copied"; }).catch(() => { info.querySelector<HTMLTextAreaElement>("#pair-elev")?.select(); }); });
    } catch (e) { fail(info, e); }
  });

  q("pair-close").addEventListener("click", () => dlg.close());

  const btn = document.createElement("button");
  btn.type = "button";
  btn.textContent = "Pair device…";
  btn.addEventListener("click", () => { addDlg.close(); dlg.showModal(); void prefill(); });
  (addDlg.querySelector("form") ?? addDlg).appendChild(btn);
  return () => { if (addDlg.open) addDlg.close(); dlg.showModal(); void prefill(); };
}
