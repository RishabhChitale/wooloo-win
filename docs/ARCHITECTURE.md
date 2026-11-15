# Architecture

wooloo-win and the Mac wooloo fork are two clients of two independent Herdr servers, linked by SSH over Tailscale. There is no cloud service and no new network listener.

## Diagram

```
  Mac                                                    Windows 11 PC
 +-------------------------------+                      +-------------------------------+
 | wooloo (Swift)                |                      | wooloo-win (Tauri)            |
 |  - UI, Spaces, Pair Device    |                      |  - UI, Spaces, Pair device    |
 |  - sync layer (Swift port,    |                      |  - sync layer: crates/        |
 |    must pass sync-vectors)    |                      |    wooloo-sync (Rust)         |
 |  - peer-bridge dispatcher     |                      |  - peer-dispatch.ps1          |
 |    (sh, ~/.local/share/       |                      |    (PowerShell dispatcher)    |
 |    wooloo/peer-bridge)        |                      |  - wooloo-core (SSH, Herdr)   |
 |        |                      |                      |        |                      |
 | own Herdr server (Unix sock)  |                      | own Herdr server (named pipe) |
 +---------------|---------------+                      +---------------|---------------+
                 | sshd (Remote Login)                    sshd (OpenSSH Server)
                 |                                                       |
                 +======== SSH over Tailscale (WireGuard, tailnet) ======+
                    Mac client -> PC sshd, PC client -> Mac sshd (both ways)
                    each sshd runs the key's forced command = the dispatcher
```

Each side:
- runs its own Herdr server and hosts its own Spaces; the other side's app attaches to it as a client through the SSH bridge (`remote-api-bridge`, `remote-client-bridge`), so live terminals come from Herdr itself;
- runs sync of wooloo's own Space metadata (see below) over the same SSH connection;
- accepts SSH only through a dispatcher that matches the request against a fixed allow-list.

## Components

| Component | Where | Role |
|---|---|---|
| `crates/wooloo-core` | Rust lib | Herdr protocol, machines.toml, connection manager, pairing code (`pairing.rs`) |
| `crates/wooloo-sync` | Rust lib + CLI | Metadata sync: HLC, LWW registers, validation, exchange protocol, state file |
| `src-tauri` | Tauri v2 app | Glue (commands/events in `INTERFACE.md`), pair flow (`pair.rs`) |
| `src` | TypeScript | Frontend |
| `resources/peer-dispatch.ps1` | PowerShell | Windows forced-command dispatcher |
| `docs/peer-allowlist.json` | data | The exact request templates; shared by the Mac and Windows dispatchers (`scripts/check-allowlist-drift.py` keeps the embedded copy in sync) |
| `docs/sync-protocol.md`, `docs/sync-vectors.json` | spec | Sync protocol v1 and the test vectors every implementation must pass |

## Pairing flow (step by step)

Prerequisites on both: Tailscale signed in to the same tailnet; the same Herdr version; sshd on (Mac: Remote Login; PC: OpenSSH Server with the Tailscale-only firewall rule, see `docs/windows-openssh-setup.md`).

1. On each machine, open Pair Device. The app creates a device key if missing (Mac: `~/.ssh/wooloo_peer_ed25519`, passphrase-less ed25519 via `ssh-keygen`; the private key never leaves the machine) and reads its SSH host public key and Herdr version.
2. The user enters this machine's SSH target (`user@host[:port]`, a Tailscale name or `100.x` address). The app shows a text code `wooloo-pair-v1:<base64url JSON>` with `target`, `publicKey`, `hostKey`, `herdrVersion`, `session`, `label`.
3. The user copies each code to the other machine (any channel) and pastes it. Decoding validates every field.
4. Each side shows the peer's label, target and key fingerprints. The user compares the fingerprints on both screens and confirms. The code is only as trustworthy as the channel it travelled over; the fingerprint comparison is the check.
5. On confirm, the app on each machine installs a dispatcher, adds the peer's public key to its authorized_keys with `command="<dispatcher> --peer <id>",restrict` (on the PC for an Administrators account: `C:\ProgramData\ssh\administrators_authorized_keys`), and writes the peer's host key and an ssh config entry `wooloo-peer-<id>` with `StrictHostKeyChecking yes`.
6. Mac only: it then runs `herdr machine add wooloo-peer-<id> ...`. If that fails the profile is still written and the sheet says so (see UNVERIFIED).
7. The apps attach to each other's Herdr servers through the bridge forms in the allow-list and start syncing metadata.

Herdr path of the peer: the pair code carries NO path field (six fields only, unknown keys are rejected).
- For a paired `wooloo-peer-<id>` machine this app sends only allow-listed requests (`wooloo-peer api|client|check <session>`); the peer's dispatcher supplies its own paired Herdr, so nothing assumes `$HOME/.local/bin/herdr`.
- A Mac peer's own `{herdr}` is the space-free symlink `~/.local/share/wooloo/peer/herdr`, baked into its dispatcher at pairing.
- Our own Herdr path is `where.exe herdr.exe` at accept time, written forward-slashed and bare into our authorized_keys line as `--herdr`.
- It is not sent in the code; the Mac uses it only for the `{herdr}` bridge scripts, which need it given out of band.
- A different, quoted or relative `{herdr}` in a request is refused with exit 64.

Removing a peer = deleting its authorized_keys line and its `wooloo_peers/<id>.*` files.

## Threat model

Assumed: both machines are the user's own; the tailnet is private; the attacker may be another tailnet member, a process on the network, or someone who obtained a peer's private key.

Controls:
- **No cloud, no new listener.** Only sshd, already on the machines, over Tailscale. The Windows firewall rule admits only `100.64.0.0/10`. Never expose SSH or Herdr to the internet.
- **Forced command + `restrict`.** The peer key can only run the dispatcher; `restrict` disables port, agent and X11 forwarding and PTY.
- **Exact allow-list.** The dispatcher never executes the request text. It must equal one of the templates in `docs/peer-allowlist.json` byte for byte (after slots are filled), then the dispatcher itself runs the paired Herdr binary. Session names are `[A-Za-z0-9_-]{1,64}`, the Herdr path is fixed at pairing time and must match `^/[A-Za-z0-9._/-]+$`, requests are capped at 16 KiB. Anything else exits 64 with `wooloo-peer-bridge: <reason>`.
- **Host key pinning.** `StrictHostKeyChecking yes` with a per-peer known_hosts file from the pairing code.
- **Sync allow-list.** Only fixed fields are accepted (see `docs/sync-protocol.md` section 4), validated on send and on receive.

A stolen peer private key CAN: connect to that machine over SSH (if it can reach the tailnet address) and use the allowed Herdr bridges, i.e. read and drive that machine's Herdr Spaces and terminals as the user. That is full terminal access to the paired Herdr session, so treat the key as that powerful. It could also write allow-listed Space metadata if sync is allowed through the same key.
It CANNOT: get a shell or run arbitrary commands, forward ports, read or write files over SSH, run `herdr machine add` or other Herdr commands outside the templates, change the allow-list or authorized_keys, or reach anything not on Tailscale (firewall scope) unless the thief is on the tailnet. Revoke by deleting the key line on the host.

Caveats: the device key has no passphrase (so BatchMode works); protect `~/.ssh` and the PC user profile. Sync notes are user text and are synced by design; do not paste secrets into Space notes. A tailnet ACL restricting who may reach port 22 is recommended but is the user's configuration.

## What sync carries (and never carries)

Only per-Space name, category, colour, order, notes, pinned tab labels, host machine + Herdr workspace id, deleted flag, and the unit-state category map. Never: secrets, keys, tokens, game art, terminal scrollback or surfaces, file paths, environment, Herdr config. Anything outside the allow-list is rejected on send and receive.

## Conflict policy (summary)

- Data is a set of registers `(key, field)`; one value per pair.
- Each write gets a hybrid logical clock stamp `(wall ms, counter)` and the writer's node id (`mac-xxxxxxxx`, `win-xxxxxxxx`).
- Per field, last writer wins: larger wall, then counter, then node id, then canonical value text. So replicas converge in any order. Different fields of one Space merge independently; the same field edited on both sides while offline keeps only the later edit (notes and pinned tabs are whole-value).
- Clear a field by writing `null`. Deleting a Space writes `deleted=true` (and clears notes and pins); delete wins over concurrent edits, only `deleted=false` with a newer stamp restores it.
- Ops more than 10 minutes ahead of the receiver's clock are deferred, not lost.
- Space identity is a UUID minted only by the hosting machine, bound to the Herdr workspace id; Herdr stays the source of truth for the label.
- Exchange: ping-pong line-delimited JSON, full document each time, each side commits only after a complete `end`; an interrupted exchange is harmless.

## UNVERIFIED

Nothing has been run on a real Windows PC or between two real machines yet. CI builds and tests only.

- Windows ConPTY: live terminal rendering, keyboard encoding, paste, resize, scrolling in the Windows app.
- The Windows local Herdr named-pipe endpoint (derived from the Herdr source; `pipe_path` override exists).
- The PC dispatcher answering Herdr's machine-add probe (answers `Linux`/`x86_64` and an `/c/...` path like a POSIX host, since Herdr rejects `Windows_NT`; the real commands still run the Windows herdr.exe). Verify with `herdr machine add` from the Mac.
- `administrators_authorized_keys`: the Pair flow writing it (needs an elevated app) and the ACL (`SYSTEM` and `Administrators` only) so sshd accepts it.
- sshd `restrict` behaviour (no PTY) with Herdr's bridges on both macOS and Windows sshd, and multi-line `SSH_ORIGINAL_COMMAND` through Windows sshd.
- Herdr's request strings are from version 0.9.5 (allow-list is `provisional`); other versions may differ. Version skew between the two Herdr builds is untested.
- Whether Herdr's own `ssh` calls honour `~/.ssh/config`'s `Include` line.
- Tailscale ACLs and firewall scoping on a real tailnet; reconnect after sleep, network loss or Tailscale restart (target: within 30 s).
- Sync over the SSH channel: `docs/peer-allowlist.json` currently has no template for `wooloo-sync exchange`, so how the exchange is carried through the forced command is not yet specified or tested. `wooloo-sync` is spec plus crate; the Swift port is not written.
- The NSIS installer installing and launching, SmartScreen flow (installer is unsigned).
- Two clients on one Space (size is last-active-wins per Herdr).
