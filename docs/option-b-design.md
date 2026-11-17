# Option B: list the paired PC's Spaces without `herdr machine add`

Status: design only (step 1). No Mac code changed. Not pushed.

## Why
Windows OpenSSH runs the first command of a multiplexed connection for every later session, and Herdr
always uses ControlMaster=auto. So `herdr machine add` (HerdrConnections.swift `manage`, WorkspaceFiles.manageMachine)
cannot work against the PC. Option B skips Herdr's machine catalog and talks to the PC through the
allow-listed bridge with fresh, non-multiplexed ssh calls.

## What `wooloo-peer api <session>` speaks
`wooloo-peer api S` = `exec herdr --session S remote-api-bridge` on the PC (docs/peer-allowlist.json, entry
wooloo-api). It is Herdr's stdio bridge to the session's `herdr.sock`, i.e. the SAME newline-delimited JSON as the
Mac's local `herdr.sock` (HerdrSocket/HerdrEventStream): `session.snapshot`, `events.subscribe` (stream; each event
-> new snapshot), one-shot commands (`workspace.create`, `tab.create`, `pane.split`, `pane.send_input`, `pane.read`,
`workspace.rename`...). `wooloo-peer client S` = `remote-client-bridge`, the binary generation-1 client endpoint
(`herdr-client.sock`) for terminal surfaces and keyboard/mouse. `wooloo-peer check S` -> `herdr-api-bridge-v1`;
`wooloo-peer hello` -> `wooloo-peer-bridge-v1 0.9.5`. Allow-list already permits all four. Both bridges write presence.

## What already exists on the Mac (main)
- HerdrRemoteBridge: per-store local unix sockets (`api`, `client`); each accepted connection spawns one ssh whose
  stdio is the socket. For peers, `PeerMachine.bridgeCommand` already maps to `wooloo-peer api|client <session>`.
  HerdrStore already runs against a `machine: HerdrMachineProfile?` through these sockets, so the store, event
  stream, snapshot, and terminal pipeline need no change.
- PeerSyncService (branch peer-sync-service): `wooloo-peer sync` via `PeerMachine.helloArguments` (no ControlMaster). Already safe on Windows.
- PeerLinkMonitor/hello: also non-multiplexed.
The multiplexing leaks in two places only: `HerdrRemoteBridge.arguments` (adds ControlMaster=auto/ControlPath/ControlPersist)
and `WorkspaceFiles.ssh` (same, for file/Git ops), plus Herdr's own `--machine` path (`remoteSnapshot`).

## Plan
1. Peer profile without Herdr: `PeerMachineStore` builds `HerdrMachineProfile(id: "peer-<id>", label, target: "wooloo-peer-<id>",
   session: <PC session, default "default">, enabled: true)` from `~/.ssh/wooloo_peers/*.conf` (read only; never touch other ~/.ssh).
   `HerdrConnectionCatalog.machines` = Herdr's list + peers (peers win on id; refresh never calls `machine add`).
   PeerPairingView stops calling `machine add` for peers and just saves the session (small JSON in app support).
2. No multiplexing for peers: in `HerdrRemoteBridge.arguments` and `WorkspaceFiles.ssh`, if `PeerMachine.isPeer(machine)` pass
   `-o ControlMaster=no -o ControlPath=none` instead. One ssh per bridge connection (api, client), matching what Windows OpenSSH needs.
   Pass `-F ~/.ssh/wooloo_peers/<id>.conf` explicitly so it works even without the Include line.
3. Read-only first: select the peer in the sidebar; HerdrStore connects through the bridge; render Spaces, tabs, panes, and
   surfaces; disable input (`HerdrStore.readOnly` guard on send/paste/mouse/create/rename/close). Needs only `api` + `client` reads. Gate on `check`/`hello` first, show the skew banner.
4. Interactive: remove the guard. Input already flows through `remote-client-bridge` (keys/paste/mouse) and `pane.send_input`; typing in either app shows in the other because Herdr on the PC owns the pane. Renames: the existing sync service publishes/reconciles labels (`publishLocal`, `reconcile`); renaming a peer Space also calls `workspace.rename` on the PC.
5. Things keyed on Herdr's profile model degrade for peers until handled: `remoteSnapshot` (use the bridge api instead), `manageMachine`/edit sheet (hide for peers), file explorer/Git over `WorkspaceFiles.ssh` (works non-multiplexed via step 2 only if the dispatcher allows it; it does not allow arbitrary shell, so Files/Git are disabled for peers).

## Risks
- Each connection pays a full SSH handshake (~100-300 ms on Tailscale); the event stream is long-lived, so acceptable. Snapshot polling must stay on the event stream, not per-call ssh.
- Windows sshd MaxSessions/MaxStartups: api + client + sync + hello = 4 concurrent connections; keep hello at 30 s and sync on its timer.
- Herdr version skew between Mac and PC (0.9.x): bridge protocol is v1, snapshot shape may differ; keep decoding fields optional, show the banner.
- ConPTY surfaces from Windows: client-bridge streams may carry Windows-specific escapes/line endings; renderer must tolerate.
- Without Herdr's catalog the app no longer self-heals peer profiles; the conf files are the source of truth.
- `-F` plus `Include` both present: harmless, but the explicit `-F` must not read user-wide config (we pass it only for peers).
- Tests: use a fake `ssh` (WorkspaceFiles.sshExecutable) and FakeHerdrServer; no real Herdr sessions, no pairing in the real app. CI-only builds.

## Size
~300-450 lines Swift + tests: PeerMachineStore (~100), arguments changes (~20), catalog merge (~30), pairing view change (~50), read-only guard (~80), tests (~150).

## Open question for commander
Should Files/Git for peers be out of scope (dispatcher forbids arbitrary shell), i.e. read-only Spaces + terminals only?
