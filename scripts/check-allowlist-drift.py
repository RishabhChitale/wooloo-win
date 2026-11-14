#!/usr/bin/env python3
"""Fail if docs/peer-allowlist.json and resources/peer-dispatch.ps1 disagree."""
import json, re, sys, pathlib
root = pathlib.Path(__file__).resolve().parent.parent
al = json.loads((root / "docs/peer-allowlist.json").read_text())
src = (root / "resources/peer-dispatch.ps1").read_text()
errs = []

verbs = set(re.findall(r"remote-[a-z]+-bridge", src))
if verbs != set(al["bridgeVerbs"]):
    errs.append(f"bridge verbs: dispatcher {sorted(verbs)} vs list {sorted(al['bridgeVerbs'])}")
flags = set(re.findall(r"--(?:idle-timeout-v1|check)\b", src))
if flags != set(al["bridgeFlags"]):
    errs.append(f"flags: dispatcher {sorted(flags)} vs list {sorted(al['bridgeFlags'])}")
m = re.search(r"wooloo-peer \(([a-z|]+)\)", src)
short = set(m.group(1).split("|")) | {"hello"} if m else set()
if short != set(al["shortForms"]):
    errs.append(f"short forms: dispatcher {sorted(short)} vs list {sorted(al['shortForms'])}")
if al["readyMarker"] not in src:
    errs.append("ready marker missing from dispatcher")
if not re.search(rf"exit {al['denyExitCode']}\b", src):
    errs.append("deny exit code missing from dispatcher")

# If the Mac repo's copy is present locally, the vendored copy must equal it.
mac = pathlib.Path.home() / "Claude/wooloo-peer-wt/docs/peer-allowlist.json"
if mac.exists():
    macj = json.loads(mac.read_text())
    if {k: v for k, v in macj.items() if not k.startswith("_")} != {k: v for k, v in al.items() if not k.startswith("_")}:
        errs.append(f"vendored copy differs from {mac}")

if errs:
    print("allow-list drift:\n  " + "\n  ".join(errs)); sys.exit(1)
print("allow-list in sync")
