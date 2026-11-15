# Status (second-commander, 2026-11-15)

Goal: Mac wooloo (Swift fork) and Windows wooloo-win (Tauri) are peers: each runs its own Herdr server, each lists the other's Spaces live over Tailscale + SSH, and Space metadata (names, categories, colours, order, notes) syncs both ways with HLC last-writer-wins. No cloud.

## Done (wooloo-win main)
- M2 client: core crate, Tauri glue, canvas frontend, CI (win+mac), live round-trip vs real herdr 0.9.5, NSIS release v0.1.0-rc1 (prerelease).
- M3 Windows half: pairing (wooloo-pair-v1), PowerShell forced-command dispatcher with the shared exact allow-list, drift check, OpenSSH setup doc, PC-TEST.md.
- M4 Rust: wooloo-sync crate + spec + vectors (CI `vectors` job green), CLI edit + sync --loopback, metadata sync in Tauri (meta.rs, set_space_meta/get_meta/sync_now/sync_status), synced-context UI, e2e workflow, Windows installer artifact on every main push.

## In flight
- probopass (1): contract decisions in INTERFACE.md (node-id host, set_space_meta object arg, presence, sync-status event, pending/versions, tombstones).
- shelmet (3): conform src to INTERFACE.md (CI contract test currently red until then), then Swift sync port (queued).
- slowpoke (7): e2e script on CLI edit + loopback.
- klang (6): wire-format vectors section.
- togekiss (11): Mac PR #1 (RishabhChitale/wooloo peer-pairing): exact-match allow-list dispatcher + 'wooloo-peer sync' entry (rev 2) + tests; then CI green.
- bellsprout (4): extends verify_vectors.py to exchange/binding when klang finishes.

## Blocked / waiting
- Opus pawns (drilbur, ditto) hit their usage limit; work moved to sonnet pawns.
- Commander session hit its limit; hold the final real-PC test until it returns.
- Mac PR #1: NOT merged, no tag, nothing installed. Order after green: merge, bump to 0.3.17.6, wait main CI green (no re-run), tag, install (backup first), smoke test Pair Device with a throwaway session.
- Allow-list `provisional` flag: flip to false on both lists together (Mac JSON first, then Windows).
- M6: Mac app lists the PC's Spaces and applies synced metadata (after the release).

## UNVERIFIED on real hardware
Windows ConPTY and ssh.exe bridge, named pipe names, Windows_NT answer to Herdr's machine-add probe, administrators_authorized_keys ACLs, sshd restrict behaviour, canvas rendering/IME, Tailscale ACLs, the Swift side (never built).
