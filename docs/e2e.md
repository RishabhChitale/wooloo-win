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
| build wooloo-sync CLI | The sync crate and its `wooloo-sync` binary compile (needs `cargo`; otherwise the whole replica scenario prints `SKIP`, as on the Mac without cargo). |
| sync A->B (<=2s) | The real `wooloo-sync sync`/`exchange` protocol carries a Space name+category from replica A's unit-categories file into replica B's. |
| rename on B reaches A (<=2s) | The reverse direction, including removal of the old name. |
| partition, edit both, heal | Edits made on both replicas while no exchange runs converge to identical files after heal (LWW on conflict). |

### Real vs simulated in the replica scenario

- Real: two Herdr servers, the `wooloo-sync` binary and its exchange protocol, per-replica state and unit-category files.
- Simulated: the SSH hop is a shim `ssh` on `PATH` that runs the remote command locally with that machine's state env (no sshd, no forced-command dispatcher, no allow-list check). The "app layer" that mirrors a Herdr Space label+category into the unit-categories file is the script's `set_cats`; the real apps do this themselves. The partition is "no exchange runs", not a network cut.

## NOT covered (need real two-machine runs)

- The SSH hop (`--remote`, `--machine`): all calls here use local Unix sockets.
- Tailscale connectivity, latency, NAT, and reconnect behavior.
- Windows-side Herdr (named pipes/OpenSSH, see `windows-openssh-setup.md`).
- Version skew between the two Herdr builds (see `src/version-skew.ts` for the pure check).
- **A real link kill / merge**: the script only simulates a partition (see above). A real run
  must cut Tailscale/SSH mid-exchange and mid-edit; `TODO(link-kill)` / `TODO(link-merge)`
  comments in the script mark where.
- The forced-command dispatcher and `docs/peer-allowlist.json` enforcement on `wooloo-sync exchange`.
