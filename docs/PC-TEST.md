# wooloo-win real-PC test session

Plain-text checklist. Anything marked UNVERIFIED was never run against a real
sshd / real PC by the pawns; if it differs, that is a finding: use the report
template at the bottom.

## TL;DR
1. Mac: wooloo 0.3.17.8 installed, Remote Login ON, Tailscale up.
2. PC: install Tailscale, Herdr 0.9.5, OpenSSH Server (steps 1-4).
3. PC: install wooloo-win from the CI artifact or release v0.1.0-rc2 (step 5).
4. Pair PC -> Mac and Mac -> PC (step 6), each side pastes the other's code.
5. Check: Space made on one side appears on the other (step 7).
6. Check: typing on one side shows on the other (step 8).
7. Check: rename syncs within ~2 s (step 9).
8. Check: Tailscale off/on reconnects within 30 s (step 10).
9. Check: version-skew banner when Herdr versions differ (step 11).
10. Anything off: fill in the FAILURE REPORT at the bottom and send it back.

## Installer
Latest main build (workflow "Build Windows", artifact wooloo-windows-nsis):
  gh run list -R RishabhChitale/wooloo-win --workflow "Build Windows" --branch main --limit 1
  gh run download <run-id> -R RishabhChitale/wooloo-win -n wooloo-windows-nsis -D wooloo-nsis
Or browse: https://github.com/RishabhChitale/wooloo-win/actions (Build Windows > latest green main run > Artifacts).
Or release (once tagged): https://github.com/RishabhChitale/wooloo-win/releases/tag/v0.1.0-rc2

## Steps (PC = Windows, run PowerShell as Administrator unless noted)

### 1. Tailscale (both machines, same tailnet)
Install from https://tailscale.com/download/windows and log in with the same account as the Mac.
  tailscale status
  tailscale ip -4
  tailscale ping <mac-magicdns-name-or-100.x>     (on PC)
  tailscale ping <pc-magicdns-name-or-100.x>      (on Mac)
Expected: pong both ways. Note both 100.x addresses.

### 2. Herdr 0.9.5 on the PC (same as the Mac)
  irm https://herdr.dev/install.ps1 | iex
  herdr --version
Expected: 0.9.5. Compare with `herdr --version` on the Mac.

### 3. OpenSSH Server
Use the block in docs/windows-openssh-setup.md. In short:
  Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
  Set-Service sshd -StartupType Automatic
  Start-Service sshd
  Remove-NetFirewallRule -Name OpenSSH-Server-In-TCP -ErrorAction SilentlyContinue
  New-NetFirewallRule -Name wooloo-sshd-tailscale -DisplayName "wooloo sshd (Tailscale only)" -Direction Inbound -Protocol TCP -LocalPort 22 -Action Allow -RemoteAddress 100.64.0.0/10
Note: the forced-command dispatcher runs under powershell.exe (Windows
PowerShell 5.1), not pwsh. The default sshd shell can stay cmd.
Expected: `Get-Service sshd` shows Running.

### 4. (Admin accounts only) authorized_keys ACL
Admin users read C:\ProgramData\ssh\administrators_authorized_keys. After
pairing (step 6) run:
  $f = "C:\ProgramData\ssh\administrators_authorized_keys"
  icacls $f /inheritance:r /grant "SYSTEM:F" /grant "Administrators:F"
UNVERIFIED: whether pairing writes this file or only prints the line to append
(the dialog shows a "needs elevation" line to run from an Administrator shell).

### 5. Install wooloo-win
Run the NSIS installer. SmartScreen: click "More info" then "Run anyway".
Launch it. Expected: main window opens with the machine rail.

### 6. Pair in BOTH directions
Mac prerequisite: System Settings > General > Sharing > Remote Login ON.

A) PC -> Mac (Mac appears on the PC)
 1. PC wooloo-win: open "Pair device..." (button; UNVERIFIED location, expected in or near the "+ Add" / "Add machine" dialog).
 2. Step 1 "Show this PC's code": enter user@100.x.y.z of THIS PC, click "Make code". Copy the wooloo-pair-v1:... code. Note the fingerprints.
 3. Mac wooloo: Connections window > "Pair Device..." > paste the PC's code into the Mac's paste field, review fingerprints match, accept.
 4. Mac shows a code of its own; copy it.
 5. PC dialog, Step 2 "Paste the other device's code": paste the Mac's code, click "Review", check fingerprints match what the Mac shows, optionally tick the ~/.ssh/config Include box, click "Accept pairing". Expected: "Paired with <label>." or the needs-elevation line (run it in Admin PowerShell, see step 4).
B) Mac -> PC: the above already exchanged both codes; if the Mac does not list the PC, repeat A with roles swapped (Mac makes code, PC reviews it). Mac target should be user@<MagicDNS or 100.x>.
Expected on Mac after pairing: ~/.ssh/wooloo_peers/<id>.conf exists and Herdr has machine wooloo-peer-<id>. If the sheet says "profile written; Herdr couldn't add it: ..." record the text (likely the Windows_NT probe risk below).

### 7. Spaces visible both ways
Create a Space on the Mac: it should appear on the PC. Create one on the PC: it should appear on the Mac.

### 8. Typing mirrors
Type in a pane on one side; the text appears on the other.

### 9. Rename sync
Rename a Space on one side. Expected on the other within ~2 s (HLC last-writer-wins per field).

### 10. Reconnect
On the PC: `tailscale down`, wait 20 s, `tailscale up`. Expected: machine rail shows disconnected, then reconnects within 30 s without restarting the app. Use the per-machine reconnect button only if it does not.

### 11. Version skew banner
Make Herdr versions differ (e.g. update/downgrade one side) and relaunch. Expected: the #skew banner appears in wooloo-win. Restore versions after.

## Known risks
- Windows_NT probe: the Windows dispatcher answers `uname -s` with "Windows_NT"
  (resources/peer-dispatch.ps1:157). Stock `herdr machine add` may reject it. UNVERIFIED.
- administrators_authorized_keys ACL: wrong permissions make sshd silently ignore keys (step 4).
- The Mac allow-list includes Herdr's own probes; UNVERIFIED against a real sshd.

## FAILURE REPORT (copy, fill in, send back)
Step number:
Expected:
Actual:
Screenshot (file/path):
herdr --version (PC):
herdr --version (Mac):
wooloo-win version / installer source (artifact run id or release):
App logs: dir %APPDATA%\app.wooloo.win (UNVERIFIED which log files exist; paste any *.log)
ssh -vvv output (Mac -> PC):   ssh -vvv <user>@<pc-100.x> true
ssh -vvv output (PC -> Mac):   ssh -vvv <user>@<mac-100.x> true
sshd event log (PC, Admin):    Get-WinEvent -LogName "OpenSSH/Operational" -MaxEvents 50 | Format-List TimeCreated,Message   (UNVERIFIED log name)
tailscale status (both):
NEVER paste private keys (e.g. ~/.ssh/wooloo_peer_ed25519). Public fingerprints are fine.

## Appendix
- Mac peer files: ~/.ssh/wooloo_peers/<id>.conf and .known_hosts; device key ~/.ssh/wooloo_peer_ed25519; dispatcher ~/.local/share/wooloo/peer-bridge; presence ~/.local/state/wooloo/peer-presence/.
- PC: dispatcher %APPDATA%\app.wooloo.win\peer-dispatch.ps1; host key C:\ProgramData\ssh\ssh_host_ed25519_key.pub.
- authorized_keys line form: command="powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ... -Peer <id>",restrict ssh-ed25519 ...
- Sync state: ~/.config/wooloo-sync/state.json (WOOLOO_SYNC_STATE overrides), via `wooloo-sync exchange`.
- Architecture: docs/ARCHITECTURE.md; allow-list: docs/peer-allowlist.json.
