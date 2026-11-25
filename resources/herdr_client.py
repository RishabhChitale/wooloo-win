#!/usr/bin/env python3
"""Generation-1 client endpoint shim for the PC's Herdr app (session 'herdr-app').
Binary frames on stdin/stdout (u32 LE length + payload), as wooloo's HerdrSurfaceStream speaks them.
Serves a text mirror of the focused tab's terminals (hc read, no colors) and forwards typed input
(hc send / hc keys). ids: pane = terminal = herdr-core address (e.g. t3j), same as herdr_compat.py."""
import json, os, queue, subprocess, sys, struct, threading, time, pathlib, uuid
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

LOGF = H / ".local/share/wooloo-peer/client.log"
def log(msg):
    try:
        if LOGF.exists() and LOGF.stat().st_size > 200000: LOGF.write_text("", encoding="utf-8")
        with open(LOGF, "a", encoding="utf-8") as f: f.write("%s %s\n" % (time.strftime("%H:%M:%S"), msg))
    except Exception:
        pass

def send(payload):
    log("send tag=%s len=%d" % (payload[0] if payload else "?", len(payload)))
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
        r = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout, stdin=subprocess.DEVNULL, creationflags=0x08000000)
        return r.returncode, r.stdout, r.stderr
    except Exception as ex:
        return 1, "", str(ex)

def tree():
    c, o, _ = run([APP, "ls"])
    try: return json.loads(o) if c == 0 and o.strip() else {"spaces": []}
    except ValueError: return {"spaces": []}

BOOT = "herdr-app-" + uuid.uuid4().hex[:8]
state = {"cols": 120, "rows": 40, "tab": None, "ws": None, "rev": 0, "last": None, "stop": False, "gen": 0, "fp": None}
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

try:
    import vt_mini
except Exception as _ex:
    vt_mini = None
    log("vt_mini unavailable: %s" % _ex)

NOWIN = 0x08000000
DIRTY = threading.Event()
_cell_cache = {}

def cell(ch, fg=0, bg=0, mod=0, skip=0):
    key = (ch, fg, bg, mod, skip)
    b = _cell_cache.get(key)
    if b is None:
        if len(_cell_cache) > 20000: _cell_cache.clear()
        b = s(ch) + num(fg) + num(bg) + num(mod) + (b"\x01" if skip else b"\x00") + b"\x00"
        _cell_cache[key] = b
    return b

def read_screen(tid, rows):
    c, o, _ = run([HC, "read", tid, "--lines", "0"])
    lines = o.replace("\r", "").split("\n") if c == 0 else ["(cannot read %s)" % tid]
    while lines and not lines[-1].strip(): lines.pop()
    return lines[-rows:]

def clean_env():
    return {k: v for k, v in os.environ.items() if not (k.upper().startswith("HC_") or "HERDR" in k.upper())}

class Observer:
    """Follows one core terminal with `hc attach --observe --json` and keeps a colored screen of it."""
    def __init__(self, tid):
        self.tid, self.proc, self.scr, self.ready = tid, None, None, False
        self.lock = threading.Lock()
        self.last_use = time.time(); self.last_start = 0; self.last_size_check = 0
        self.size = (81, 49)
        self.start()

    def term_size(self):
        c, o, _ = run([HC, "get", self.tid, "--json"], timeout=10)
        try:
            g = json.loads(o); return int(g.get("cols") or 81), int(g.get("rows") or 49)
        except Exception:
            return self.size

    def start(self):
        self.last_start = time.time()
        self.size = self.term_size()
        with self.lock:
            self.scr = vt_mini.Screen(*self.size); self.ready = False
        try:
            self.proc = subprocess.Popen([HC, "attach", "--observe", "--json", self.tid], stdout=subprocess.PIPE,
                                         stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, env=clean_env(), creationflags=NOWIN)
        except Exception as ex:
            log("observer start %s: %s" % (self.tid, ex)); self.proc = None; return
        threading.Thread(target=self.reader, args=(self.proc, self.scr), daemon=True).start()

    def reader(self, proc, scr):
        try:
            while True:
                chunk = proc.stdout.read1(65536)
                if not chunk: break
                with self.lock:
                    scr.feed(chunk)
                    if scr is self.scr: self.ready = True
                DIRTY.set()
        except Exception as ex:
            log("observer read %s: %s" % (self.tid, ex))
        finally:
            if scr is self.scr: self.ready = False
            DIRTY.set()

    def alive(self):
        return self.proc is not None and self.proc.poll() is None

    def stop(self):
        try:
            if self.proc: self.proc.kill()
        except Exception:
            pass

    def maintain(self):
        now = time.time()
        if not self.alive():
            if now - self.last_start > 2: self.start()
        elif now - self.last_size_check > 5:
            self.last_size_check = now
            if self.term_size() != self.size:
                self.stop(); self.start()

OBS = {}

def ensure_observers(terms):
    if vt_mini is None or (H / '.local/share/wooloo-peer/no-obs').exists(): return
    now = time.time()
    for tid in terms:
        o = OBS.get(tid)
        if o is None:
            OBS[tid] = Observer(tid)
        else:
            o.last_use = now; o.maintain()
    for tid in [t for t in OBS if t not in terms and now - OBS[t].last_use > 30]:
        OBS.pop(tid).stop()

def pane_cells(tid, w, rows, inner_w):
    """Rows of cells (symbol, fg, bg, mod, skip) for one pane, plus its cursor (x, y) or None."""
    o = OBS.get(tid)
    if o is not None and o.ready:
        with o.lock:
            scr = o.scr
            off = max(0, scr.rows - rows)
            out = []
            for y in range(min(rows, scr.rows)):
                row = scr.grid[off + y]
                line = row[:inner_w]
                if line and line[-1][4] == 0 and len(line) == inner_w and len(row) > inner_w and row[inner_w][4] == 1:
                    line = line[:-1] + [(" ", 0, 0, 0, 0)]
                out.append(line)
            cur = None
            if scr.cursor_visible and 0 <= scr.y - off < rows and scr.x < inner_w:
                cur = (scr.x, scr.y - off)
        return out, cur
    lines = read_screen(tid, rows)
    return [[(ch if ch.isprintable() and ch else " ", 0, 0, 0, 0) for ch in line[:inner_w]] for line in lines], None

def build_surface(terms, cols, rows):
    n = max(1, len(terms))
    widths = [cols // n] * n; widths[-1] += cols - sum(widths)
    blank = (" ", 0, 0, 0, 0)
    grid = [[blank] * cols for _ in range(rows)]
    panes, x0, cursor = [], 0, None
    fp = state.get("fp")
    focus = fp if fp in terms else (terms[0] if terms else None)
    for tid, w in zip(terms, widths):
        inner_w = max(1, w - (1 if n > 1 else 0))
        pcells, cur = pane_cells(tid, w, rows, inner_w)
        for y, line in enumerate(pcells):
            if y >= rows: break
            grid[y][x0:x0 + len(line)] = line
        if cur and tid == focus: cursor = (x0 + cur[0], cur[1])
        panes.append((tid, x0, w))
        x0 += w
    body = b"".join(cell(*c) for row in grid for c in row)
    state["sig"] = hash((body, tuple(terms), cols, rows, cursor, focus))
    state["rev"] += 1
    out = num(13) + s(BOOT) + num(state["rev"]) + num(state["rev"]) + num(cols * rows) + body
    out += num(cols) + num(rows)
    out += (b"\x01" + num(cursor[0]) + num(cursor[1]) + b"\x01" + b"\x00") if cursor else b"\x00"
    out += num(0) + num(0)                           # hyperlinks, legacy graphics
    out += num(len(panes))
    for tid, x, w in panes:
        rect = num(x) + num(0) + num(w) + num(rows)
        out += s(tid) + num(state["rev"]) + rect + rect + b"\x00" + b"\x00"
        out += (b"\x01" if tid == focus else b"\x00") + b"\x00\x00\x00" + num(0) + num(0)
    out += num(0)                                    # splits
    out += b"\x00"                                   # popup
    out += num(0) + num(0) + num(0)                  # graphics: delivered, placements, retained
    return out

def pump():
    last_key, terms, last_tree, gen_seen, last_send = None, [], 0, -1, 0
    while not state["stop"]:
        try:
            DIRTY.wait(0.15); DIRTY.clear()
            now = time.time()
            if state.get("gen", 0) != gen_seen or now - last_tree > 2.0:
                t = tree(); gen_seen = state.get("gen", 0); last_tree = now
                with slock:
                    terms = tab_terminals(t) or []
                ensure_observers(terms)
            if not terms: continue
            gap = 0.1 - (time.time() - last_send)
            if gap > 0: time.sleep(gap)
            with slock: cols, rows = state["cols"], state["rows"]
            frame = build_surface(terms, cols, rows)
            if state["sig"] != last_key:
                last_key = state["sig"]; send(frame); last_send = time.time()
            if any(not (OBS.get(t) and OBS[t].ready) for t in terms): time.sleep(0.5)
        except (BrokenPipeError, OSError):
            return
        except Exception as ex:
            log("pump: %s" % ex); time.sleep(0.5)

INQ = queue.Queue()

def kq(pane, names):
    INQ.put((pane, list(names)))

def key_worker():
    while True:
        pane, names = INQ.get()
        try:
            while True:
                try: p2, n2 = INQ.get(timeout=0.012)
                except queue.Empty: break
                if p2 == pane and len(names) + len(n2) <= 64: names += n2
                else:
                    run([HC, "keys", pane] + names); pane, names = p2, list(n2)
            run([HC, "keys", pane] + names)
        except Exception as ex:
            log("keys: %s" % ex)

threading.Thread(target=key_worker, daemon=True).start()


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
        for i in range(0, len(keys), 64): kq(pane, keys[i:i + 64])
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
            kq(pane, [name.lower() if name.isalpha() else name])
        else:
            kq(pane, [(pre + name).lower()])


WAKE_PY = str(H / ".local/bin/unit-wake.py")
_woke = {}

def wake_space_for(tab_id=None, ws_id=None):
    """A client focused a tab or Space: wake that Space's hibernated agents (idempotent; at most once a minute per Space)."""
    def work():
        try:
            t = tree()
            name = None
            for sp in t.get("spaces", []):
                if (ws_id and sp["id"] == ws_id) or (tab_id and any(tb["id"] == tab_id for tb in sp["tabs"])):
                    name = sp["name"]; break
            if not name or time.time() - _woke.get(name, 0) < 60: return
            _woke[name] = time.time()
            log("wake check for Space %s" % name)
            run([sys.executable, WAKE_PY, name.lower() + " /"], timeout=240)
        except Exception as ex:
            log("wake error %s" % ex)
    threading.Thread(target=work, daemon=True).start()

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
            log("recv tag=%s len=%d" % (tag, ln))
            if tag == 20:
                kind, data = r.string(), r.string()
                log("  control %s %s" % (kind, data[:160]))
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
                DIRTY.set()
            elif tag == 15:                          # JSON request (tab/workspace focus)
                r.string(); req = json.loads(r.string()); p = req.get("params") or {}
                log("  request %s %s" % (req.get("method"), json.dumps(p)[:160]))
                with slock:
                    if req.get("method") == "tab.focus": state["tab"] = p.get("tab_id"); state["ws"] = None
                    elif req.get("method") == "workspace.focus": state["ws"] = p.get("workspace_id"); state["tab"] = None
                if req.get("method") == "pane.focus": state["fp"] = p.get("pane_id")
                if req.get("method") in ("tab.focus", "workspace.focus", "pane.focus"): state["gen"] += 1
                DIRTY.set()
                if req.get("method") == "tab.focus": wake_space_for(tab_id=p.get("tab_id"))
                elif req.get("method") == "workspace.focus": wake_space_for(ws_id=p.get("workspace_id"))
                state["last"] = None
            elif tag == 13:                          # pane input
                handle_input(r)
        except Exception as ex:
            sys.stderr.write("frame: %s\n" % ex); sys.stderr.flush()
    state["stop"] = True

if __name__ == "__main__":
    try:
        main()
        log("main loop ended (stdin closed)")
    except BaseException as ex:
        import traceback
        log("main crashed: %s" % traceback.format_exc().replace(chr(10), " | "))
        raise
