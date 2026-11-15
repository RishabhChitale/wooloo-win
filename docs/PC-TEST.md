# wooloo-win FINAL real-PC test (rc5) - 10-minute checklist

Plain text. UNVERIFIED = never run on the real PC/sshd by the pawns; if it
differs, that is a finding (see FAILURE REPORT).
Button names are the exact strings from the code (see table at the end).

## Facts
PC  = rish-win11, user risha (whoami: rish-win11\risha), Tailscale 100.104.240.58
Mac = user zomato, Tailscale 100.121.45.79 (rishabhs-macbook-pro.tail7fd3ee.ts.net)
Already DONE: Tailscale both, Herdr 0.9.5 both, sshd running on PC, Mac Remote Login ON.
PC Herdr: C:\Users\risha\.herdr\packages\standalone\releases\0.9.5-x86_64-pc-windows-msvc\herdr.exe
Mac needs wooloo 0.3.17.9 or newer.

## Checklist
1. PC: install rc5 over rc4. Download:
   https://github.com/RishabhChitale/wooloo-win/releases/download/v0.1.0-rc5/wooloo_0.1.0_x64-setup.exe
   Run it (SmartScreen: More info > Run anyway). Expect: installs over rc4.
2. PC: open wooloo-win. Local machine dot should be GREEN.
   Red: click the reload arrow (title "Reconnect" on a machine row).
   Click "Start Herdr" ONLY if no server is running.
3. PC: click "Pair device" (rail header, next to "+ Add"; also the link icon titled
   "Pair device" on a machine row). Dialog title "Pair device", heading
   "1. Show this PC's code". The target box (placeholder "this PC as the peer reaches
   it: user@100.x.y.z") is prefilled risha@100.104.240.58 (editable). Click "Generate code".
   Expect: "Send this code to the other device (and compare fingerprints there):"
   with the wooloo-pair-v1:... code, then "key <fp>", "host <fp>", "Herdr: <path>".
   (Dialog is opened from the same buttons; the Add-machine dialog also has "Pair device...".)
4. Copy the PC's code to the Mac (any way).
5. Mac: wooloo > Connections window > "Pair Device..." (title "Pair Device").
   Paste the PC's code in the unlabeled paste box; the line "<label> . <target> . session
   <s> . Herdr <v>" plus "Device key <fp>" and "Host key <fp>" appear: compare with the
   PC's key/host lines. Click "Pair".
   Expect success; Mac Herdr gets machine wooloo-peer-<id>.
   If it says "profile written; Herdr couldn't add it: ..." copy that text (see Risks).
6. Mac: field "Reach this Mac at (user@host[:port])" -> zomato@100.121.45.79, click
   "Generate Code" then "Copy Code". Copy it to the PC.
7. PC: "Pair device" dialog, heading "2. Enter a code": paste into the box (placeholder
   "wooloo-pair-v1:..."), click "Review", compare fingerprints, optionally tick "Add an
   Include line to ~/.ssh/config so plain ssh also knows this peer", click "Accept pairing".
   Expect "Paired with <label>."
   If instead "Paired, but this PC's <path> still needs the peer's key": open PowerShell
   as Administrator and paste the "Or paste this into PowerShell run as Administrator:"
   box (or click "Copy commands"), once. Then "Close". UNVERIFIED on the real PC.
8. Expected results:
   a. PC rail shows the Mac machine with a GREEN dot and its Spaces.
   b. Mac sidebar shows the PC machine and its Spaces.
   c. Open the same Space on both; type on one; text appears on the other.
   d. Rename a Space on one side: appears on the other within ~2 s.
      EXPECTED-IF-SYNC-WIRED (M7 sync); if not wired, note it, not a failure.
   e. PC: `tailscale down`, wait 20 s, `tailscale up`: rail shows disconnected, then
      reconnects within 30 s without restarting the app.
   f. Version-skew banner appears ONLY if the two Herdr versions differ (both 0.9.5 now: expect none).

## Troubleshooting by symptom
Red dot (local): Herdr server not running -> reload, then "Start Herdr". Check
  `herdr --version` works in PowerShell. Still red: Copy diagnostics.
Red dot (Mac machine): Tailscale down on either side (`tailscale ping 100.121.45.79`
  from PC; `tailscale ping 100.104.240.58` from Mac); Mac Remote Login off; Mac asleep.
Bad target: must be user@100.x.y.z, here exactly risha@100.104.240.58 (not
  rish-win11\risha, not a hostname with spaces).
Access denied / Permission denied (publickey): step 3 elevated commands not run, or
  administrators_authorized_keys ACL wrong. Rerun them as Administrator, then
  `Restart-Service sshd`.
"Access is denied (os error 5)" on the local dot: the Herdr server was started from an
  Administrator window. Never start herdr from an Administrator window. The app shows: "The Herdr
  server was started by a different (elevated) user context. Stop it with: herdr server stop (in the
  window where you started it) and click Start Herdr". Start Herdr also runs `herdr server stop`
  itself when the server is running but its pipe is denied.
herdr not found: PC path above missing from PATH or the pairing; run
  `& "C:\Users\risha\.herdr\packages\standalone\releases\0.9.5-x86_64-pc-windows-msvc\herdr.exe" --version`.
Host key prompt (ssh asks "continue connecting?"): from a plain ssh test, answer yes
  once; if the app hangs on it, pairing did not record the host key: report it.
tailscale ping works but ssh times out (TCP 22 blocked while the Tailscale path is fine):
  RULED OUT on this PC: Shields Up is false; rule wooloo-sshd-tailscale is
  Enabled/Allow/Any; sshd listens on 0.0.0.0:22 and ::. (Elsewhere: check with
  tailscale debug prefs | findstr /i ShieldsUp; fix tailscale set --shields-up=false.)
  Decision tree (PowerShell, in order, stop at the first that fails):
  1. PC -> itself:  Test-NetConnection 100.104.240.58 -Port 22
     Fails -> sshd/local problem: Get-Service sshd; netstat -an | findstr ":22 "
     Succeeds -> sshd is fine, go on.
  2. PC -> Mac:     Test-NetConnection 100.121.45.79 -Port 22
     Fails -> Mac side (Remote Login, Mac firewall) or Tailscale ACL; test from the Mac
     with: nc -vz 100.104.240.58 22 to compare directions.
  3. Windows firewall: Get-NetFirewallProfile | Format-Table Name,Enabled,DefaultInboundAction
     and inbound blocks: Get-NetFirewallRule -Direction Inbound -Action Block -Enabled True |
     Format-Table DisplayName,Profile. A Block rule beats Allow; also check third-party
     security software.
  4. Tailscale ACLs (admin console > Access controls): must allow this device to reach
     the other on tcp:22 (tags, grants). Look at the ACL "Preview" for both machines.
  Paste every result into the FAILURE REPORT. UNVERIFIED results pending from the user.
Never start herdr from an Administrator window (see the Administrator note above).

## Known risks
- Windows dispatcher answers `uname -s` with "Windows_NT" (resources/peer-dispatch.ps1);
  stock `herdr machine add` may reject it. UNVERIFIED.
- Mac allow-list vs real sshd probes: UNVERIFIED.

## FAILURE REPORT (fill in, send back)
Step number:
Expected:
Actual:
Screenshot:
Copy diagnostics from wooloo-win (PC), paste:
Copy diagnostics from wooloo (Mac), paste:
ssh -vvv from Mac:   ssh -vvv risha@100.104.240.58 true
ssh -vvv from PC:    ssh -vvv zomato@100.121.45.79 true
PC, Admin PowerShell: Get-Service sshd
PC, Admin PowerShell: Get-WinEvent -LogName "OpenSSH/Operational" -MaxEvents 50 | Format-List TimeCreated,Message   (UNVERIFIED log name)
herdr --version on PC:
herdr --version on Mac:
NEVER paste private keys (e.g. ~/.ssh/wooloo_peer_ed25519).

## Appendix
- Mac: ~/.ssh/wooloo_peers/<id>.conf + .known_hosts; key ~/.ssh/wooloo_peer_ed25519; dispatcher ~/.local/share/wooloo/peer-bridge.
- PC: dispatcher %APPDATA%\app.wooloo.win\peer-dispatch.ps1; host key C:\ProgramData\ssh\ssh_host_ed25519_key.pub.
- authorized_keys form: command="powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ... -Peer <id>",restrict ssh-ed25519 ...
- Sync state ~/.config/wooloo-sync/state.json via `wooloo-sync exchange`. See docs/ARCHITECTURE.md, docs/windows-openssh-setup.md.

## Exact UI strings used
PC (wooloo-win)
| String | Source |
| Pair device (rail button, dialog title, row icon title) | src/main.ts:17,88; docs/pair-ui-strings.txt |
| + Add (title "Add machine") | src/main.ts:17 |
| Start Herdr | src/main.ts:67 |
| Copy diagnostics | src/main.ts:70 |
| Reconnect (row button, arrow icon) | src/main.ts:88 |
| 1. Show this PC's code / 2. Enter a code | docs/pair-ui-strings.txt |
| Generate code, Review, Accept pairing, Close, Copy commands | docs/pair-ui-strings.txt |
| this PC as the peer reaches it: user@100.x.y.z; wooloo-pair-v1:... | docs/pair-ui-strings.txt |
| Add an Include line to ~/.ssh/config so plain ssh also knows this peer | docs/pair-ui-strings.txt |
| Paired with <label>. / Paired, but this PC's <path> still needs the peer's key | docs/pair-ui-strings.txt |
Mac (wooloo)
| Connections (window title) | HerdrConnectionsView.swift:77 |
| Pair Device... (button) | HerdrConnectionsView.swift:82 |
| Pair Device (sheet title) | PeerPairingView.swift:26 |
| Reach this Mac at (user@host[:port]) | PeerPairingView.swift:32 |
| Generate Code, Copy Code, Pair, Close | PeerPairingView.swift:35,37,73,70 |
| Device key <fp>, Host key <fp> | PeerPairingView.swift:98-99 |
