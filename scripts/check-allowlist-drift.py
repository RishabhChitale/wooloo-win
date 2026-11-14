#!/usr/bin/env python3
"""Keep docs/peer-allowlist.json (vendored from the Mac repo) and the blob embedded in
resources/peer-dispatch.ps1 identical. `--write` regenerates the embedded blob.
Also compares with the Mac copy when it is present locally."""
import json, pathlib, re, sys

root = pathlib.Path(__file__).resolve().parent.parent
doc = root / "docs/peer-allowlist.json"
ps1 = root / "resources/peer-dispatch.ps1"
RX = re.compile(r"(\$alJson = @'\n)(.*?)(\n'@\n)", re.S)


def slim(al):
    return {
        "marker": al["marker"],
        "markerPrefix": al["markerPrefix"],
        "entries": [{k: e[k] for k in ("name", "transport", "template")} for e in al["entries"]],
    }


al = json.loads(doc.read_text())
want = slim(al)
src = ps1.read_text()
m = RX.search(src)
if not m:
    sys.exit("embedded allow-list block not found in peer-dispatch.ps1")
blob = json.dumps(want, separators=(",", ":"), ensure_ascii=True)
if "'@" in blob:
    sys.exit("blob would end the here-string")

if "--write" in sys.argv:
    ps1.write_text(src[: m.start(2)] + blob + src[m.end(2):])
    print("embedded allow-list written")
    sys.exit(0)

errs = []
try:
    have = json.loads(m.group(2))
except ValueError as e:
    have = None
    errs.append(f"embedded blob is not JSON: {e}")
if have is not None and have != want:
    names_a = [e["name"] for e in want["entries"]]
    names_b = [e["name"] for e in have.get("entries", [])]
    errs.append(f"embedded blob differs from docs/peer-allowlist.json (vendored {len(names_a)} entries, embedded {len(names_b)}); run scripts/check-allowlist-drift.py --write")

mac = pathlib.Path.home() / "Claude/wooloo-peer-wt/docs/peer-allowlist.json"
if mac.exists() and json.loads(mac.read_text()) != al:
    errs.append(f"vendored copy differs from {mac}")

if errs:
    print("allow-list drift:\n  " + "\n  ".join(errs)); sys.exit(1)
print(f"allow-list in sync ({len(want['entries'])} entries)")
