#!/usr/bin/env python3
"""Capture a real herdr generation-1 client stream into tests/fixtures/*.bin.

Only talks to a throwaway session (default: wooloo-win-test). Start it first:
  herdr --session wooloo-win-test server
Each saved frame is the raw wire frame: 4-byte LE length + payload.
"""
import json, os, socket, struct, sys, time

SESSION = os.environ.get("WOOLOO_TEST_SESSION", "wooloo-win-test")
if SESSION == "wooloo":
    sys.exit("refusing to touch a real session")
DIR = os.path.expanduser(f"~/.config/herdr/sessions/{SESSION}")
OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "wooloo-core", "tests", "fixtures")
COLS, ROWS = 100, 30


def num(v):
    if v < 251: return bytes([v])
    if v < 1 << 16: return b"\xfb" + struct.pack("<H", v)
    if v < 1 << 32: return b"\xfc" + struct.pack("<I", v)
    return b"\xfd" + struct.pack("<Q", v)

def string(s):
    b = s.encode(); return num(len(b)) + b

def control(kind, data): return num(20) + string(kind) + string(data)
def frame(p): return struct.pack("<I", len(p)) + p

def api(method, params=None):
    s = socket.socket(socket.AF_UNIX); s.connect(os.path.join(DIR, "herdr.sock")); s.settimeout(3)
    s.sendall((json.dumps({"id": "1", "method": method, "params": params or {}}) + "\n").encode())
    b = b""
    while b"\n" not in b: b += s.recv(65536)
    return json.loads(b)

def read_frame(s):
    h = b""
    while len(h) < 4:
        c = s.recv(4 - len(h))
        if not c: return None
        h += c
    n = struct.unpack("<I", h)[0]; p = b""
    while len(p) < n: p += s.recv(n - len(p))
    return h + p

def main():
    snap = api("session.snapshot")["result"]["snapshot"]
    if not snap["workspaces"]: api("workspace.create", {"label": "fx", "focus": True}); snap = api("session.snapshot")["result"]["snapshot"]
    pane = snap["panes"][0]["pane_id"]
    os.makedirs(OUT, exist_ok=True)
    json.dump(snap, open(os.path.join(OUT, "snapshot.json"), "w"), indent=1)
    s = socket.socket(socket.AF_UNIX); s.connect(os.path.join(DIR, "herdr-client.sock")); s.settimeout(2)
    hello = {"generation": 1, "cell_width_px": 8, "cell_height_px": 16, "surface_size": {"cols": COLS, "rows": ROWS},
             "pixel_mouse": False, "direct_graphics": False, "endpoint_keybindings": False, "mouse_capture": False,
             "surface_active": True, "surface_reuse": False, "surface_delta": False, "surface_scroll": True,
             "snapshot_codecs": ["shell.snapshot.v1"], "surface_codecs": ["shell.surface.v1"],
             "input_codecs": ["shell.input.semantic.v1"], "blob_codecs": ["shell.blob.v1"]}
    s.sendall(frame(control("endpoint.hello.v1", json.dumps(hello))))
    frames = []
    def drain(t=1.5):
        end = time.time() + t
        while time.time() < end:
            try: f = read_frame(s)
            except socket.timeout: break
            if f is None: break
            frames.append(f)
    drain(2)
    cmd = "printf '\\033[1mbold\\033[0m \\033[3mital\\033[0m \\033[4munder\\033[0m \\033[7minv\\033[0m \\033[31mred\\033[0m \\033[38;2;10;200;30mrgb\\033[0m \\033[44mbluebg\\033[0m \\344\\270\\255\\346\\226\\207 caf\\303\\251\\n'"
    api("pane.send_input", {"pane_id": pane, "text": cmd, "keys": ["Enter"]})
    drain(2)
    for i in range(40): api("pane.send_input", {"pane_id": pane, "text": f"echo line{i}", "keys": ["Enter"]})
    drain(2)
    for i, f in enumerate(frames):
        open(os.path.join(OUT, f"frame_{i:03d}.bin"), "wb").write(f)
    print(f"saved {len(frames)} frames; pane {pane}")

main()
