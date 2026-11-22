#!/usr/bin/env python3
"""Generation-1 client endpoint shim for the PC's Herdr app (session 'herdr-app').
Binary frames on stdin/stdout (u32 LE length + payload), as wooloo's HerdrSurfaceStream speaks them.
Serves a text mirror of the focused tab's terminals (hc read, no colors) and forwards typed input
(hc send / hc keys). ids: pane = terminal = herdr-core address (e.g. t3j), same as herdr_compat.py."""
import json, subprocess, sys, struct, threading, time, pathlib, uuid
H = pathlib.Path.home()
APP = str(H / "AppData/Local/Programs/herdr-app/herdr-app.exe")
HC = str(H / "AppData/Local/Programs/herdr-core/hc.exe")
OUT = sys.stdout.buffer
IN = sys.stdin.buffer
wlock = threading.Lock()

def num(v):
    if v < 251: return bytes([v])
    if v <= 0xFFFF: return b"\xfb" + struct.pack("<H", v)
    if v <= 0xFFFFFFFF: return b"\xfc" + struct.pack("<I", v)
    return b"\xfd" + struct.pack("<Q", v)

def s(v):
    b = v.encode("utf-8")
    return num(len(b)) + b

def send(payload):
    with wlock:
        OUT.write(struct.pack("<I", len(payload)) + payload); OUT.flush()

def control(kind, data): return num(20) + s(kind) + s(data)

class Reader:
    def __init__(self, b): self.b, self.i = b, 0
    def byte(self): v = self.b[self.i]; self.i += 1; return v
    def number(self):
        t = self.byte()
        if t < 251: return t
        n = {251: 2, 252: 4, 253: 8}[t]
        v = int.from_bytes(self.b[self.i:self.i + n], "little"); self.i += n; return v
    def string(self):
        n = self.number(); v = self.b[self.i:self.i + n].decode("utf-8", "replace"); self.i += n; return v

def run(cmd, timeout=30):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
        return r.returncode, r.stdout, r.stderr
    except Exception as ex:
        return 1, "", str(ex)

def tree():
    c, o, _ = run([APP, "ls"])
    try: return json.loads(o) if c == 0 and o.strip() else {"spaces": []}
    except ValueError: return {"spaces": []}

BOOT = "herdr-app-" + uuid.uuid4().hex[:8]
state = {"cols": 120, "rows": 40, "tab": None, "ws": None, "rev": 0, "last": None, "stop": False}
slock = threading.Lock()

def tab_terminals(t):
    sp_active = None
    if state["tab"]:
        for sp in t["spaces"]:
            for tb in sp["tabs"]:
                if tb["id"] == state["tab"]: return [x["terminal"] for x in tb["terminals"]]
    if state["ws"]:
        sp_active = next((sp for sp in t["spaces"] if sp["id"] == state["ws"]), None)
    if sp_active:
        tb = next((tb for tb in sp_active["tabs"] if tb.get("active")), sp_active["tabs"][0] if sp_active["tabs"] else None)
        if tb: return [x["terminal"] for x in tb["terminals"]]
    return []

def cell(ch):
    return s(ch) + num(0) + num(0) + num(0) + b"\x00" + b"\x00"

def read_screen(tid, rows):
    c, o, _ = run([HC, "read", tid, "--lines", "0"])
    lines = o.replace("\r", "").split("\n") if c == 0 else ["(cannot read %s)" % tid]
    while lines and not lines[-1].strip(): lines.pop()
    return lines[-rows:]

def build_surface(terms, cols, rows):
    n = max(1, len(terms))
    widths = [cols // n] * n; widths[-1] += cols - sum(widths)
    grid = [[" "] * cols for _ in range(rows)]
    panes, x0 = [], 0
    for tid, w in zip(terms, widths):
        inner_w = max(1, w - (1 if n > 1 else 0))
        for y, line in enumerate(read_screen(tid, rows)):
            col = 0
            for ch in line:
                if col >= inner_w: break
                grid[y][x0 + col] = ch if len(ch.encode("utf-8")) and ch.isprintable() else " "
                col += 1
        panes.append((tid, x0, w))
        x0 += w
    body = b"".join(cell(ch) for row in grid for ch in row)
    state["sig"] = hash((body, tuple(terms), cols, rows))
    state["rev"] += 1
    out = num(13) + s(BOOT) + num(state["rev"]) + num(state["rev"]) + num(cols * rows) + body
    out += num(cols) + num(rows) + b"\x00"          # no cursor
    out += num(0) + num(0)                           # hyperlinks, legacy graphics
    out += num(len(panes))
    for i, (tid, x, w) in enumerate(panes):
        rect = num(x) + num(0) + num(w) + num(rows)
        out += s(tid) + num(state["rev"]) + rect + rect + b"\x00" + b"\x00"
        out += (b"\x01" if i == 0 else b"\x00") + b"\x00\x00\x00" + num(0) + num(0)
    out += num(0)                                    # splits
    out += b"\x00"                                   # popup
    out += num(0) + num(0) + num(0)                  # graphics: delivered, placements, retained
    return out

def pump():
    last_key = None
    while not state["stop"]:
        try:
            t = tree()
            with slock:
                terms = tab_terminals(t) or ([] if state["tab"] or state["ws"] else [])
                cols, rows = state["cols"], state["rows"]
            if terms:
                frame = build_surface(terms, cols, rows)
                if state["sig"] != last_key:
                    last_key = state["sig"]; send(frame)
        except (BrokenPipeError, OSError):
            return
        except Exception as ex:
            sys.stderr.write("pump: %s\n" % ex); sys.stderr.flush()
        time.sleep(0.8)

KEYS = {0: "backspace", 1: "enter", 2: "left", 3: "right", 4: "up", 5: "down", 6: "home", 7: "end",
        8: "pageup", 9: "pagedown", 10: "tab", 11: "shift+tab", 12: "delete", 14: "esc"}

def chars_to_keys(txt):
    out = []
    for ch in txt.replace("\r\n", "\n"):
        if ch == "\n": out.append("enter")
        elif ch == "\t": out.append("tab")
        elif ch == " ": out.append("space")
        elif ch.isascii() and ch.isalpha() and ch.isupper(): out.append("shift+" + ch.lower())
        elif ch.isascii() and ch.isprintable(): out.append(ch)
    return out

def handle_input(r):
    pane = r.string(); r.number()                    # id, ClientInput version
    kind = r.number()
    if kind in (1, 3):                               # text / paste: typed as keys (hc send would press Enter)
        keys = chars_to_keys(r.string())
        for i in range(0, len(keys), 64): run([HC, "keys", pane] + keys[i:i + 64])
    elif kind == 0:                                  # key
        code = r.number(); ch = None
        if code == 15:
            b0 = r.b[r.i]; n = 1 if b0 < 0x80 else 2 if b0 < 0xE0 else 3 if b0 < 0xF0 else 4
            ch = r.b[r.i:r.i + n].decode("utf-8", "replace"); r.i += n
        mods = r.byte()
        name = KEYS.get(code, ch)
        if not name: return
        pre = ("ctrl+" if mods & 2 else "") + ("alt+" if mods & 4 else "") + ("shift+" if mods & 1 and code != 11 else "")
        if code == 15 and not (mods & 6):
            run([HC, "send", pane, name] if False else [HC, "keys", pane, name.lower() if name.isalpha() else name])
        else:
            run([HC, "keys", pane, (pre + name).lower()])

def main():
    pumper = None
    while True:
        h = IN.read(4)
        if len(h) < 4: break
        ln = struct.unpack("<I", h)[0]
        b = IN.read(ln)
        if len(b) < ln: break
        r = Reader(b)
        try:
            tag = r.number()
            if tag == 20:
                kind, data = r.string(), r.string()
                if kind == "endpoint.hello.v1":
                    hello = json.loads(data)
                    sz = hello.get("surface_size") or {}
                    state["cols"], state["rows"] = int(sz.get("cols", 120)), int(sz.get("rows", 40))
                    send(control("endpoint.welcome.v1", json.dumps({
                        "generation": 1, "error": None, "snapshot_codec": "shell.snapshot.v1",
                        "surface_codec": "shell.surface.v1", "input_codec": "shell.input.semantic.v1",
                        "blob_codec": "shell.blob.v1", "capabilities": ["health_check"]})))
                    send(control("shell.snapshot.v1", json.dumps({"boot_id": BOOT})))
                    pumper = threading.Thread(target=pump, daemon=True); pumper.start()
                elif kind == "endpoint.health.ping.v1":
                    send(control("endpoint.health.pong.v1", data))
            elif tag == 12:                          # resize
                r.number(); r.number()
                with slock: state["cols"], state["rows"] = r.number(), r.number()
            elif tag == 15:                          # JSON request (tab/workspace focus)
                r.string(); req = json.loads(r.string()); p = req.get("params") or {}
                with slock:
                    if req.get("method") == "tab.focus": state["tab"] = p.get("tab_id"); state["ws"] = None
                    elif req.get("method") == "workspace.focus": state["ws"] = p.get("workspace_id"); state["tab"] = None
                state["last"] = None
            elif tag == 13:                          # pane input
                handle_input(r)
        except Exception as ex:
            sys.stderr.write("frame: %s\n" % ex); sys.stderr.flush()
    state["stop"] = True

if __name__ == "__main__": main()
