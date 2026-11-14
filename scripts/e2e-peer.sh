#!/usr/bin/env bash
# Two-"machine" Herdr e2e: two throwaway sessions, driven from separate processes.
# See docs/e2e.md for what this does and does not prove.
set -u

A=wooloo-e2e-a
B=wooloo-e2e-b
HERDR=${HERDR:-herdr}
LOGDIR=${TMPDIR:-/tmp}
LOGDIR=${LOGDIR%/}
FAILS=0

# Only these two sessions may ever be touched.
guard() { case "$1" in "$A"|"$B") ;; *) echo "refusing to touch session '$1'" >&2; exit 2 ;; esac; }
h() { local s=$1; shift; guard "$s"; "$HERDR" --session "$s" "$@"; }

now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }

cleanup() {
  for s in "$A" "$B"; do
    guard "$s"
    "$HERDR" --session "$s" server stop >/dev/null 2>&1 || true
  done
  sleep 1
  for s in "$A" "$B"; do
    "$HERDR" session delete "$s" >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT INT TERM

command -v jq >/dev/null || { echo "jq required" >&2; exit 2; }
command -v "$HERDR" >/dev/null || { echo "herdr not found" >&2; exit 2; }

# Refuse to run if our sessions already exist and are running (could be someone's work).
for s in "$A" "$B"; do
  if "$HERDR" session list 2>/dev/null | awk -v n="$s" '$1==n && $2=="running"{f=1} END{exit !f}'; then
    echo "session $s already running; refusing" >&2; trap - EXIT; exit 2
  fi
done

step() { # step <name> <start_ms> <ok 0/1> [detail]
  local dt=$(( $(now_ms) - $2 ))
  if [ "$3" -eq 0 ]; then echo "PASS  ${1} (${dt} ms)"; else echo "FAIL  ${1} (${dt} ms) ${4:-}"; FAILS=$((FAILS+1)); fi
}

# poll <timeout_s> <cmd...>: retry every 0.1s until cmd succeeds
poll() {
  local limit=$1; shift
  local end=$(( $(now_ms) + limit * 1000 ))
  while :; do
    if "$@"; then return 0; fi
    [ "$(now_ms)" -ge "$end" ] && return 1
    sleep 0.1
  done
}

# --- 0. boot both servers ---
t=$(now_ms)
for s in "$A" "$B"; do
  guard "$s"
  nohup "$HERDR" --session "$s" server >"$LOGDIR/herdr-$s.log" 2>&1 &
done
boot_ok() { h "$A" api snapshot >/dev/null 2>&1 && h "$B" api snapshot >/dev/null 2>&1; }
poll 30 boot_ok; step "boot two sessions" "$t" $? "(see $LOGDIR/herdr-$A.log)"
[ "$FAILS" -eq 0 ] || exit 1

# --- 1. create Space on A ---
t=$(now_ms)
out=$(h "$A" workspace create --cwd "$PWD" --label e2e-space 2>&1)
WS=$(printf '%s' "$out" | jq -r '.result.workspace.workspace_id // empty')
PANE=$(printf '%s' "$out" | jq -r '.result.root_pane.pane_id // empty')
[ -n "$WS" ] && [ -n "$PANE" ]; step "create Space on A" "$t" $? "$out"
[ "$FAILS" -eq 0 ] || exit 1

# --- 2. separate client process sees it within 2s ---
t=$(now_ms)
sees_space() { h "$A" api snapshot 2>/dev/null | jq -e --arg w "$WS" --arg l "$1" \
  '[.result.snapshot.workspaces[]? | select(.workspace_id==$w and .label==$l)] | length==1' >/dev/null; }
poll 2 sees_space e2e-space; step "peer client sees new Space (<=2s)" "$t" $?

# --- 3. send input on A, read back from another process ---
t=$(now_ms)
h "$A" pane run "$PANE" 'echo e2e-$((20+22))' >/dev/null 2>&1
reads_back() { h "$A" pane read "$PANE" 2>/dev/null | grep -q '^e2e-42$'; }
poll 5 reads_back; step "send input, read output from other process" "$t" $?

# --- 4. rename, visible from other process ---
t=$(now_ms)
h "$A" workspace rename "$WS" e2e-renamed >/dev/null 2>&1
poll 2 sees_space e2e-renamed; step "rename visible from other process (<=2s)" "$t" $?

# --- 5. isolation: B must not see A's Space ---
t=$(now_ms)
h "$B" api snapshot 2>/dev/null | jq -e '[.result.snapshot.workspaces[]?] | length==0' >/dev/null
step "session B independent of A (own server)" "$t" $?

# --- TODO hooks (need real two-machine runs / sync layer; see docs/e2e.md) ---
echo "SKIP  TODO link-kill: drop A<->B link mid-edit (needs wooloo-sync + two machines)"
echo "SKIP  TODO link-merge: restore link, assert metadata converges (needs wooloo-sync)"
# TODO(link-kill):  kill_link() { :; }   # e.g. tailscale down / pf rule / ssh -O exit
# TODO(link-merge): assert_converged() { :; }  # compare wooloo-sync state on both sides

[ "$FAILS" -eq 0 ] && echo "ALL PASS" || echo "$FAILS step(s) FAILED"
[ "$FAILS" -eq 0 ]
