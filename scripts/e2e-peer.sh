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

# --- Part 2: two wooloo-sync replicas (docs/sync-protocol.md) ---
# Each "machine" = one Herdr session + one sync replica (own state file and unit
# categories file). The "ssh hop" is a local shim: the real `wooloo-sync sync`
# runs unchanged, but `ssh -- <A|B> cmd` just runs cmd locally with that
# machine's state env. No real SSH, no Tailscale.
SYNC_DIR=$(mktemp -d "$LOGDIR/wooloo-e2e-sync.XXXXXX")
trap 'cleanup; rm -rf "$SYNC_DIR"' EXIT INT TERM
SYNC_BIN=""
if command -v cargo >/dev/null 2>&1; then
  t=$(now_ms)
  ( cargo build -q -p wooloo-sync --bin wooloo-sync ) >"$LOGDIR/wooloo-e2e-build.log" 2>&1
  rc=$?
  step "build wooloo-sync CLI" "$t" $rc "(see $LOGDIR/wooloo-e2e-build.log)"
  if [ $rc -eq 0 ]; then
    SYNC_BIN="$(pwd)/${CARGO_TARGET_DIR:-target}/debug/wooloo-sync"
    [ -x "$SYNC_BIN" ] || SYNC_BIN=$(cargo metadata --format-version 1 --no-deps 2>/dev/null | jq -r '.target_directory')/debug/wooloo-sync
  fi
else
  echo "SKIP  wooloo-sync replica scenario: cargo not found (runs in CI)"
fi

if [ -n "$SYNC_BIN" ] && [ -x "$SYNC_BIN" ]; then
  mkdir -p "$SYNC_DIR/bin" "$SYNC_DIR/A" "$SYNC_DIR/B"
  cat >"$SYNC_DIR/bin/ssh" <<'SHIM'
#!/usr/bin/env bash
# Test shim: `ssh [opts] -- <A|B> <cmd>` runs <cmd> locally as machine <A|B>.
while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do shift; done
shift; tgt=$1; shift
case "$tgt" in A|B) ;; *) echo "e2e ssh shim: bad target $tgt" >&2; exit 255 ;; esac
export WOOLOO_SYNC_STATE="$E2E_SYNC_DIR/$tgt/state.json"
export WOOLOO_SYNC_UNITCAT="$E2E_SYNC_DIR/$tgt/unitcat.json"
exec bash -c "$1"
SHIM
  chmod +x "$SYNC_DIR/bin/ssh"
  export E2E_SYNC_DIR="$SYNC_DIR"

  # Run the real CLI as machine $1 (initiator) against machine $2 (responder).
  do_sync() {
    PATH="$SYNC_DIR/bin:$PATH" \
    WOOLOO_SYNC_STATE="$SYNC_DIR/$1/state.json" WOOLOO_SYNC_UNITCAT="$SYNC_DIR/$1/unitcat.json" \
      "$SYNC_BIN" sync "$2" --remote-cmd "$SYNC_BIN exchange" >>"$LOGDIR/wooloo-e2e-sync.log" 2>&1
  }
  # The "app layer": mirror a Herdr label + category into the unit-categories file.
  # $1 machine, then name/category pairs; replaces the whole file.
  set_cats() {
    local m=$1; shift
    local args=() ; while [ "$#" -gt 0 ]; do args+=(--arg "$1" "$2"); shift 2; done
    jq -n '$ARGS.named' "${args[@]}" >"$SYNC_DIR/$m/unitcat.json"
  }
  cat_of() { jq -r --arg n "$2" '.[$n] // empty' "$SYNC_DIR/$1/unitcat.json" 2>/dev/null; }
  has_cat() { [ "$(cat_of "$1" "$2")" = "$3" ]; }

  # 6. Space created on A (Herdr workspace e2e-renamed, category "claude") reaches B within 2s
  t=$(now_ms)
  set_cats A e2e-renamed claude
  do_sync A B && has_cat B e2e-renamed claude && [ $(( $(now_ms) - t )) -le 2000 ]
  step "sync A->B: Space name+category on B (<=2s)" "$t" $?

  # 7. rename on B appears on A
  t=$(now_ms)
  set_cats B e2e-renamed2 claude
  do_sync B A && has_cat A e2e-renamed2 claude && ! has_cat A e2e-renamed claude && [ $(( $(now_ms) - t )) -le 2000 ]
  step "rename on B reaches A (<=2s)" "$t" $?

  # 8. partition: edit both sides with no exchange (the "link" is just not running do_sync)
  t=$(now_ms)
  set_cats A e2e-renamed2 school only-a misc
  sleep 0.05
  set_cats B e2e-renamed2 creative only-b finance
  # heal (TODO(link-kill)/TODO(link-merge): replace "not calling do_sync" with a real
  # link drop between two machines in a real run, see docs/e2e.md)
  do_sync A B && do_sync B A && do_sync A B
  conv() { cmp -s <(jq -S . "$SYNC_DIR/A/unitcat.json") <(jq -S . "$SYNC_DIR/B/unitcat.json") \
    && has_cat A only-a misc && has_cat A only-b finance && [ -n "$(cat_of A e2e-renamed2)" ]; }
  conv; step "partition, edit both, heal: replicas converge" "$t" $?
  echo "INFO  conflicting edit resolved (LWW) to: $(cat_of A e2e-renamed2)"
fi

[ "$FAILS" -eq 0 ] && echo "ALL PASS" || echo "$FAILS step(s) FAILED"
[ "$FAILS" -eq 0 ]
