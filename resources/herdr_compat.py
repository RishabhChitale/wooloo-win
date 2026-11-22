#!/usr/bin/env python3
"""Answers the Herdr socket API (JSON lines on stdin/stdout) from the PC's Herdr app + herdr-core.
Used by the peer dispatcher's `wooloo-api` so the Mac sees the PC's real Spaces. ids: workspace=space id,
tab=tab id, pane=terminal=herdr-core address (e.g. t3j)."""
import json, subprocess, sys, pathlib, time, threading
H = pathlib.Path.home()
APP = str(H / "AppData/Local/Programs/herdr-app/herdr-app.exe")
HC = str(H / "AppData/Local/Programs/herdr-core/hc.exe")

def run(cmd, timeout=60):
    r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    return r.returncode, r.stdout, r.stderr

def tree():
    c, o, _ = run([APP, "ls"])
    return json.loads(o) if c == 0 and o.strip() else {"spaces": []}

def status(s): return s or "unknown"
def agg(ts):
    order = ["blocked", "done", "working", "idle"]
    st = [t.get("status") for t in ts if t.get("status")]
    for k in order:
        if k in st: return k
    return "unknown"

def workspaces(t):
    out = []
    for i, sp in enumerate(t["spaces"], 1):
        terms = [x for tb in sp["tabs"] for x in tb["terminals"]]
        act = next((tb["id"] for tb in sp["tabs"] if tb.get("active")), sp["tabs"][0]["id"] if sp["tabs"] else None)
        out.append({"workspace_id": sp["id"], "number": i, "label": sp["name"], "focused": bool(sp.get("active")),
                    "pane_count": len(terms), "tab_count": len(sp["tabs"]), "active_tab_id": act, "agent_status": agg(terms)})
    return out

def tabs(t):
    out = []
    for sp in t["spaces"]:
        for n, tb in enumerate(sp["tabs"], 1):
            out.append({"tab_id": tb["id"], "workspace_id": sp["id"], "number": n, "label": tb["name"],
                        "focused": bool(tb.get("active")), "pane_count": len(tb["terminals"]), "agent_status": agg(tb["terminals"])})
    return out

def panes(t):
    out = []
    for sp in t["spaces"]:
        for tb in sp["tabs"]:
            for x in tb["terminals"]:
                p = {"pane_id": x["terminal"], "terminal_id": x["terminal"], "workspace_id": sp["id"], "tab_id": tb["id"],
                     "focused": False, "cwd": sp.get("folder"), "agent_status": status(x.get("status"))}
                if x.get("app"): p["agent"] = x["app"]
                out.append(p)
    return out

def roster_names():
    try: return {e["address"]: e["role"] for e in json.loads((H / ".local/share/unit-strip/roster.json").read_text())}
    except Exception: return {}

def agents(t):
    names = roster_names()
    out = []
    for sp in t["spaces"]:
        for tb in sp["tabs"]:
            for x in tb["terminals"]:
                if not x.get("app"): continue
                out.append({"terminal_id": x["terminal"], "name": names.get(x["terminal"]) or x.get("name"), "agent": x["app"], "agent_status": status(x.get("status")),
                            "workspace_id": sp["id"], "tab_id": tb["id"], "pane_id": x["terminal"], "focused": False,
                            "interactive_ready": True, "cwd": sp.get("folder")})
    return out

def resolve(t, target):
    for a in agents(t):
        if target in (a["terminal_id"], a.get("name")): return a["terminal_id"]
    return target

def layouts(t):
    out = []
    for sp in t["spaces"]:
        for tb in sp["tabs"]:
            n = max(len(tb["terminals"]), 1); w = 200
            ps = [{"pane_id": x["terminal"], "rect": {"x": i * w // n, "y": 0, "width": w // n, "height": 50}}
                  for i, x in enumerate(tb["terminals"])]
            out.append({"tab_id": tb["id"], "area": {"x": 0, "y": 0, "width": w, "height": 50}, "panes": ps, "zoomed": False})
    return out

def snapshot(t):
    ws, tb, pn = workspaces(t), tabs(t), panes(t)
    fw = next((w["workspace_id"] for w in ws if w["focused"]), ws[0]["workspace_id"] if ws else None)
    ft = next((x["tab_id"] for x in tb if x["focused"] and x["workspace_id"] == fw), None)
    return {"workspaces": ws, "tabs": tb, "panes": pn, "agents": agents(t), "layouts": layouts(t),
            "focused_workspace_id": fw, "focused_tab_id": ft, "focused_pane_id": None}

def handle(m, p):
    if m == "ping":
        return {"type": "pong", "version": "0.9.5", "capabilities": {"endpoint_protocol_generation": 1}}
    t = tree()
    if m == "session.snapshot": return {"type": "session_snapshot", "snapshot": snapshot(t)}
    if m == "pane.get":
        pn = next((x for x in panes(t) if x["pane_id"] == p.get("pane_id")), None)
        if pn is None: raise ValueError("pane_not_found")
        return {"type": "pane_info", "pane": pn}
    if m == "pane.send_input":
        tid = p.get("pane_id")
        if p.get("text"):
            c, o, e = run([HC, "send", tid, p["text"].rstrip("\n")] if p["text"].endswith("\n") else [HC, "send", tid, p["text"]])
            if c: raise RuntimeError(e or o)
        if p.get("keys"):
            c, o, e = run([HC, "keys", tid] + [str(k).lower() for k in p["keys"]])
            if c: raise RuntimeError(e or o)
        return {"type": "ok"}
    if m == "workspace.list": return {"type": "workspace_list", "workspaces": workspaces(t)}
    if m == "tab.list": return {"type": "tab_list", "tabs": tabs(t)}
    if m == "pane.list": return {"type": "pane_list", "panes": panes(t)}
    if m == "agent.list": return {"type": "agent_list", "agents": agents(t)}
    if m == "workspace.create":
        args = {}
        if p.get("label"): args["name"] = p["label"]
        if p.get("cwd"): args["folder"] = p["cwd"]
        c, o, e = run([APP, "do", "space.new", json.dumps(args)])
        if c: raise RuntimeError(e or o)
        res = json.loads(o); t2 = tree()
        ws = next((w for w in workspaces(t2) if w["workspace_id"] == res["space"]), None)
        tb = next((x for x in tabs(t2) if x["tab_id"] == res.get("tab")), None)
        rp = next((x for x in panes(t2) if x["pane_id"] == res.get("terminal")), {"pane_id": res.get("terminal"), "terminal_id": res.get("terminal"), "workspace_id": res["space"], "tab_id": res.get("tab"), "focused": False, "agent_status": "unknown"})
        rp.setdefault("scroll", {"offset_from_bottom": 0, "max_offset_from_bottom": 0, "viewport_rows": 40}); rp.setdefault("revision", 0)
        return {"type": "workspace_created", "workspace": ws, "tab": tb, "root_pane": rp}
    if m == "workspace.rename":
        c, o, e = run([APP, "do", "space.rename", json.dumps({"id": p["workspace_id"], "name": p["label"]})])
        if c: raise RuntimeError(e or o)
        return {"type": "ok"}
    if m == "workspace.close":
        c, o, e = run([APP, "do", "space.close", json.dumps({"id": p["workspace_id"], "confirmed": True})])
        if c: raise RuntimeError(e or o)
        return {"type": "ok"}
    if m in ("pane.read", "agent.read"):
        tid = p.get("pane_id") or resolve(t, p.get("target", ""))
        c, o, e = run([HC, "read", tid, "--lines", str(p.get("lines", 40))])
        if c: raise RuntimeError(e or o)
        return {"type": "pane_read", "read": {"pane_id": tid, "source": p.get("source", "recent"), "format": "text", "text": o}}
    if m in ("pane.send_text", "agent.prompt"):
        tid = p.get("pane_id") or resolve(t, p.get("target", ""))
        text = p.get("text", "")
        if m == "agent.prompt" and not text: raise ValueError("empty_agent_prompt")
        if m == "agent.prompt": c, o, e = run([HC, "send", tid, text])
        else:
            c, o, e = run([HC, "keys", tid] + ["enter"]) if text == "\n" else run([HC, "send", tid, text.rstrip("\n")]) if text.endswith("\n") else (run([HC, "paste", tid, text]) if False else run([HC, "send", tid, text]))
        if c: raise RuntimeError(e or o)
        return {"type": "ok"}
    if m == "pane.send_keys":
        tid = p.get("pane_id") or resolve(t, p.get("target", ""))
        keys = p.get("keys") or []
        c, o, e = run([HC, "keys", tid] + [str(k).lower() for k in keys])
        if c: raise RuntimeError(e or o)
        return {"type": "ok"}
    if m == "agent.wait":
        tid = resolve(t, p.get("target", ""))
        c, o, e = run([HC, "wait", tid], timeout=max(5, int(p.get("timeout_ms", 60000) / 1000) + 5))
        return {"type": "agent_wait", "target": tid, "status": "done" if c == 0 else "timeout"}
    raise NotImplementedError(m)

def subscribe(rid):
    """Acknowledge, then emit an event whenever the PC's tree changes (polled); ends when stdin closes."""
    stop = threading.Event()
    def watch():
        try:
            for _ in sys.stdin: pass
        finally: stop.set()
    threading.Thread(target=watch, daemon=True).start()
    sys.stdout.write(json.dumps({"id": rid, "result": {"type": "subscription_started"}}) + "\n"); sys.stdout.flush()
    last = json.dumps(tree(), sort_keys=True)
    while not stop.wait(1.5):
        cur = json.dumps(tree(), sort_keys=True)
        if cur != last:
            last = cur
            sys.stdout.write(json.dumps({"event": "workspace.updated"}) + "\n"); sys.stdout.flush()

def main():
    for line in sys.stdin:
        line = line.strip()
        if not line: continue
        try:
            req = json.loads(line)
        except ValueError:
            continue
        rid = req.get("id")
        if req.get("method") == "events.subscribe":
            subscribe(rid); return
        try:
            out = {"id": rid, "result": handle(req.get("method", ""), req.get("params") or {})}
        except NotImplementedError as ex:
            out = {"id": rid, "error": {"code": "unknown_method", "message": f"not supported by the Herdr-app bridge: {ex}"}}
        except Exception as ex:
            out = {"id": rid, "error": {"code": "internal", "message": str(ex)[:300]}}
        sys.stdout.write(json.dumps(out) + "\n"); sys.stdout.flush()

if __name__ == "__main__": main()
