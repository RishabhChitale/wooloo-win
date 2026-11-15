#!/usr/bin/env bash
# Interop fixtures between the Rust crate and the Swift app (wooloo PeerSyncInteropTests).
# Neither toolchain is guaranteed locally; CI normally runs this. Usage:
#   scripts/sync-interop.sh check        # run the interop tests (assert against committed transcripts)
#   scripts/sync-interop.sh regenerate   # rewrite interop-rust.transcript from this crate
#   scripts/sync-interop.sh pull-swift <path-to-interop-swift.transcript>   # install the Swift side's transcript
# See docs/sync-protocol.md, "Interop fixtures".
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-check}" in
  check) cargo test -p wooloo-sync --test interop ;;
  regenerate) WOOLOO_INTEROP_REGENERATE=1 cargo test -p wooloo-sync --test interop rust_transcript_matches_fixture -- --nocapture ;;
  pull-swift) cp "${2:?path to interop-swift.transcript}" crates/wooloo-sync/tests/fixtures/interop-swift.transcript ;;
  *) echo "usage: $0 check|regenerate|pull-swift <file>" >&2; exit 2 ;;
esac
