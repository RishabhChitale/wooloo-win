# wooloo-win

A Windows (and macOS-buildable) Tauri client for [Herdr](https://herdr.dev). It attaches over SSH to Herdr servers on your machines (for example a Mac and a Windows PC), lists their Spaces grouped by machine, and shows live terminals. Each machine runs its own Herdr server; this app is a client for any of them. Inspired by the Swift macOS wooloo app.

## Setup

1. Install [Tailscale](https://tailscale.com) on both machines and sign in to the same tailnet.
2. Install the **same Herdr version** on both machines. On Windows, in PowerShell:
   `irm https://herdr.dev/install.ps1 | iex`
3. Enable the **OpenSSH Client** on Windows (Settings > System > Optional features).
4. To **host from the PC**: enable the **OpenSSH Server** optional feature, start the `sshd` service (set it to Automatic), and allow inbound TCP 22 in Windows Firewall (ideally only on the Tailscale interface/network):
   `New-NetFirewallRule -Name sshd -DisplayName "OpenSSH Server" -Enabled True -Direction Inbound -Protocol TCP -LocalPort 22 -Action Allow`
5. In wooloo, **add a machine** by its SSH target (e.g. `user@my-mac` using its Tailscale name). Keys and auth stay with OpenSSH.

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
