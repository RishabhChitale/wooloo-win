# Peer e2e (`scripts/e2e-peer.sh`)

Simulates two machines with two throwaway Herdr sessions, `wooloo-e2e-a` and
`wooloo-e2e-b`, each with its own server and socket. The script refuses to touch
any other session and always stops and deletes both on exit. Needs `herdr`, `jq`, `python3`.
CI: `.github/workflows/e2e.yml` (macos-latest, on push to `main` and manual dispatch).

## What each step proves

| Step | Proves |
|---|---|
| boot two sessions | Two independent Herdr servers run side by side (the "each machine runs its own server" model). |
| create Space on A | The workspace API creates a Space and returns its ids. |
| peer client sees Space (<=2s) | A second, separate process connecting to A's socket (`api snapshot`) sees the new Space quickly: the multi-client view an app gets of a remote server. |
| send input / read back | `pane run` from one process and `pane read` from another round-trip terminal I/O. |
| rename visible (<=2s) | Mutations by one client reach other clients of the same server. |
| B independent of A | Sessions don't leak state into each other (no accidental shared state). |

## NOT covered (need real two-machine runs)

- The SSH hop (`--remote`, `--machine`): all calls here use local Unix sockets.
- Tailscale connectivity, latency, NAT, and reconnect behavior.
- Windows-side Herdr (named pipes/OpenSSH, see `windows-openssh-setup.md`).
- Version skew between the two Herdr builds (see `src/version-skew.ts` for the pure check).
- **Link kill / merge with the sync layer** (`crates/wooloo-sync`): partition the link
  mid-edit on both sides, restore it, assert metadata converges. The script prints
  `SKIP  TODO link-kill` / `link-merge` and has marked hook comments
  (`TODO(link-kill)`, `TODO(link-merge)`) where `kill_link` and `assert_converged` go.
