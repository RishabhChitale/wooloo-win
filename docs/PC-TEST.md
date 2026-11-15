# wooloo-win FINAL real-PC test (rc5) - 10-minute checklist

Plain text. UNVERIFIED = never run on the real PC/sshd by the pawns; if it
differs, that is a finding (see FAILURE REPORT).
Button names below follow the rc5 flow; older builds say "Make code" instead
of "Generate code" (UNVERIFIED label match).

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
   Red: click reload (the circular arrow). Click "Start Herdr" ONLY if no server is running.
3. PC: click "Pair device" (rail header button, or the link icon on a machine row).
   Target defaults to risha@100.104.240.58 (editable). Click "Generate code".
   Expect: a copy box with the pairing code (wooloo-pair-v1:...) and fingerprints.
   If the app is NOT elevated it also shows the authorized_keys line plus PowerShell
   commands: open PowerShell as Administrator, paste them exactly as shown, once.
   (Admin accounts use C:\ProgramData\ssh\administrators_authorized_keys; the commands
   also fix its ACL.) UNVERIFIED on the real PC.
4. Copy the PC's code to the Mac (clipboard/chat/AirDrop-text, any way).
5. Mac: wooloo > Connections > Pair Device > paste the PC's code > Review.
   Check the fingerprints match the PC's dialog > Pair.
   Expect: success; Mac Herdr gets a machine wooloo-peer-<id>.
   If it says "profile written; Herdr couldn't add it: ..." copy that text (see Risks).
6. Mac shows its own code. Copy it to the PC.
7. PC: Pair dialog > "Enter a code from another device" > paste Mac's code > Review >
   compare fingerprints > Accept pairing. Expect "Paired with <label>."
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
herdr not found: PC path above missing from PATH or the pairing; run
  `& "C:\Users\risha\.herdr\packages\standalone\releases\0.9.5-x86_64-pc-windows-msvc\herdr.exe" --version`.
Host key prompt (ssh asks "continue connecting?"): from a plain ssh test, answer yes
  once; if the app hangs on it, pairing did not record the host key: report it.

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
