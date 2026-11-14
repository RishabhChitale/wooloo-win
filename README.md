# wooloo-win

A Windows (and macOS-buildable) Tauri client for [Herdr](https://herdr.dev). Each of your machines runs its own Herdr server and hosts its own Spaces. wooloo-win attaches over SSH to those servers (for example a Mac and a Windows PC), lists their Spaces grouped by machine and shows live terminals. It pairs with the Swift macOS wooloo fork as a peer over Tailscale + SSH, with no cloud service, and replicates wooloo's own Space metadata (names, categories, colours, order, notes, pinned tabs) between the two apps.

See `docs/ARCHITECTURE.md` for the diagram, pairing flow, threat model, conflict policy and the list of unverified items.

## How it fits together

- Transport: Tailscale (private tailnet) + the SSH both machines already run. No new listener, no cloud.
- Access control: a paired peer's key is limited by a forced command and `restrict` to a dispatcher that matches each request against the exact allow-list in `docs/peer-allowlist.json`. It cannot get a shell.
- Sync: `crates/wooloo-sync`, hybrid-logical-clock last-writer-wins per field (`docs/sync-protocol.md`, `docs/sync-vectors.json`). Never carries secrets or terminal scrollback.
- Real-PC test checklist: `docs/PC-TEST.md`.

## Setup

1. Install [Tailscale](https://tailscale.com) on both machines and sign in to the same tailnet.
2. Install the **same Herdr version** on both machines. On Windows, in PowerShell:
   `irm https://herdr.dev/install.ps1 | iex`
3. Windows PC: install OpenSSH Server and a Tailscale-only firewall rule as in `docs/windows-openssh-setup.md` (run as admin). Mac: turn on Remote Login.
4. Install wooloo-win from the GitHub release (below), then use **Pair device** on both apps and compare the fingerprints. Steps: `docs/ARCHITECTURE.md`. Mac side: the Mac doc `docs/peer-pairing.md` in the wooloo fork.
5. You can also add a machine by hand by its SSH target (e.g. `user@my-mac`, its Tailscale name). Keys and auth stay with OpenSSH.

## Installing the release

The installer is **unsigned**. Windows SmartScreen will warn: click **More info > Run anyway**.

## UNVERIFIED on real Windows

Nothing here has been run on a real Windows machine or against a second Herdr host yet. Unverified:
- The NSIS installer builds, installs and launches.
- SSH attach to a Mac Herdr server and to a Windows Herdr server (OpenSSH Server setup above).
- Herdr remote bridge behaviour and protocol compatibility across Herdr versions.
- Local Windows named-pipe endpoint. Derived from the herdr source (`\\.\pipe\` + the socket file path under `%APPDATA%\herdr`, `-client` for the binary channel), but the docs do not state it and it has not been tried on a real server. If it fails, set `pipe_path` on the machine in `machines.toml` to the full API socket path (for example `C:\Users\me\AppData\Roaming\herdr\herdr.sock`); debug herdr builds use `herdr-dev` instead of `herdr`.
- Keyboard encoding, paste, resize and scrolling in live terminals.
- Two clients on one Space (size is last-active-wins per Herdr).
- Reconnect behaviour after sleep or network loss.

## Development

Layout: `crates/wooloo-core` (Rust library), `src-tauri` (Tauri v2 app), `src` (Vite + TypeScript frontend). See `INTERFACE.md` for the core/frontend contract.

```
pnpm install
pnpm build       # frontend
pnpm test
pnpm tauri dev   # needs Rust stable
cargo test --workspace
```

CI (GitHub Actions) builds and tests on windows-latest and macos-latest. Pushing a `vX.Y.Z` tag builds the NSIS installer and attaches it to a GitHub release.

## License

Apache-2.0, see `LICENSE`.
