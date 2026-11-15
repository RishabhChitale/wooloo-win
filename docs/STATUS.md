# Status (second-commander, 2026-11-15)

Goal: Mac wooloo (Swift fork) and Windows wooloo-win (Tauri) are peers: each runs its own Herdr server, each lists the other's Spaces live over Tailscale + SSH, and Space metadata (names, categories, colours, order, notes) syncs both ways with HLC last-writer-wins. No cloud.

## Done (wooloo-win main)
- M2 client: core crate, Tauri glue, canvas frontend, CI (win+mac), live round-trip vs real herdr 0.9.5, NSIS release v0.1.0-rc1 (prerelease).
- M3 Windows half: pairing (wooloo-pair-v1), PowerShell forced-command dispatcher with the shared exact allow-list, drift check, OpenSSH setup doc, PC-TEST.md.
- M4 Rust: wooloo-sync crate + spec + vectors (CI `vectors` job green), CLI edit + sync --loopback, metadata sync in Tauri (meta.rs, set_space_meta/get_meta/sync_now/sync_status), synced-context UI, e2e workflow, Windows installer artifact on every main push.

## Done since last update
- Windows dispatcher caps (1 MiB/line, 32 MiB total, exit 65) green on Windows PowerShell 5.1 (the sshd default shell); the cap-boundary red was a test-harness BOM, not the dispatcher. pwsh 7 leg best-effort.
- Allow-list: revision 2, 20 entries, provisional:false, identical on Mac and Windows; drift check in CI.
- Mac: PR #1 (pairing + exact-match dispatcher + wooloo-peer sync) merged, 0.3.17.6 released and installed. PR #2 (space-free stable Herdr symlink for the bridge) merged.
- wooloo-win: sync hello carries herdr_version; skew banner live; e2e proves create/see/type/rename/partition/heal/converge (CI run 43664511661).

## In flight
- togekiss (11): release 0.3.17.7 (includes PR #2), install, Pair Device sheet check with a fake code.
- slowpoke (7): M6 Mac lists and opens the PC's Spaces (branch peer-machines).
- shelmet (3): Swift sync port (branch sync-swift) passing docs/sync-vectors.json in XCTest.
- probopass (1): pwsh 7 leg / CI cleanups.

## Blocked / waiting
- Opus pawns (drilbur, ditto, klang) hit their usage limit earlier; sonnet pawns continue.
- The real-PC test (docs/PC-TEST.md) waits for the user at the PC.
- Mac metadata sync UI (names/categories/colours) waits for the Swift sync port, then a follow-up on top of M6.

## Next
1. M6 PR green and released; 2. Swift sync port PR green; 3. wire sync into the Mac app over the 'wooloo-peer sync' request; 4. real-PC test with the user.

## UNVERIFIED on real hardware
Windows ConPTY and ssh.exe bridge, named pipe names, Windows_NT answer to Herdr's machine-add probe, administrators_authorized_keys ACLs, sshd restrict behaviour, canvas rendering/IME, Tailscale ACLs, Swift side live pairing (only CI/XCTest so far).

## Update 2026-11-15
- Mac 0.3.17.8 released and installed (PR #3 / M6: paired peers as machines, state dot, skew banner). Windows v0.1.0-rc2 prerelease published. Real-PC test: waiting for the user's go (do not start PC/Remote Login steps).
- Known flaky upstream test: MarkdownPreviewRenderingTests.testLargeTableKeepsScrollingAcrossFocusAndTypographyChanges (failed once on main CI, green on re-dispatch). Upstream test: not touched. Rule: tag only after the main push's CI is green; if red, dispatch ci.yml, wait for green, then re-run Release.
- Limits: no presence badge for remote Spaces (Herdr exposes no attach data); sidebar with no peers not screenshot-verified.
- Last functional piece: Swift sync port (shelmet) and Mac metadata sync UI.

## WHAT I NEED FROM YOU (one screen; updated 2026-11-15)
You have ~10 minutes of work at the PC and Mac once docs/PC-TEST.md is final. Everything else is automated and green in CI.
1. PC: download and run the rc5 installer (link in docs/PC-TEST.md; SmartScreen: More info > Run anyway), over rc4.
2. PC: open wooloo. The local dot should be green by itself (reload if red). Click Pair device; target should read risha@100.104.240.58; click Generate code.
3. PC: if the dialog shows an elevated PowerShell block, paste it into an Administrator PowerShell once.
4. Mac: wooloo > Connections > Pair Device > paste the PC's code > check the fingerprints > Pair. Then copy the Mac's code to the PC's Pair dialog.
5. Look: PC shows the Mac machine and its Spaces; Mac shows the PC machine and its Spaces. Open one Space on both, type, rename a Space: it should follow within a few seconds.
6. If anything fails: use Copy diagnostics on both apps and paste them to me (template at the bottom of docs/PC-TEST.md).
Nothing else is needed from you: no passwords, no keys, no tags.

## Done in this session (Windows)
rc1 to rc4 shipped; rc5 adds: robust herdr path resolution without PATH (registry, installer dirs, where.exe), local session discovery, retry loop that never gives up, session picker, Pair target editable, ssh-keyscan host key (no admin read).
