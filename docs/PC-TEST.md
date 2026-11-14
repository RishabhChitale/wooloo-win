# 10-minute real PC test (Mac <-> Windows 11)

You need: the Mac, the Windows 11 PC, one Tailscale account, the PC's admin login. "Admin PowerShell" = Start menu, type PowerShell, right-click, Run as administrator.

**Known risk (provisional):** the PC's dispatcher answers Herdr's machine-add probe as `Windows_NT`, so step 8 (`herdr machine add` on the Mac for the PC) may fail. Fallback is in step 8.

## Setup

1. **Tailscale on both.** Install from tailscale.com, sign in to the same account on Mac and PC.
   Check: on the Mac run `tailscale status` and find the PC; on the PC run `tailscale ip` and note the `100.x` address and the machine name.
   Expected: each machine lists the other.
2. **Ping.** Mac: `tailscale ping <pc-name>`. PC (PowerShell): `tailscale ping <mac-name>`.
   Expected: `pong from ... via ...` on both. If it fails, stop: fix Tailscale first.
3. **Same Herdr version.** On the PC (PowerShell): `irm https://herdr.dev/install.ps1 | iex`. Then on BOTH machines: `herdr --version`.
   Expected: identical version numbers. If they differ, update the older one.
4. **OpenSSH Server on the PC.** Admin PowerShell:
   ```powershell
   Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
   Add-WindowsCapability -Online -Name OpenSSH.Client~~~~0.0.1.0
   Set-Service -Name sshd -StartupType Automatic
   Start-Service sshd
   Remove-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -ErrorAction SilentlyContinue
   New-NetFirewallRule -Name 'wooloo-sshd-tailscale' -DisplayName 'OpenSSH (Tailscale only)' `
     -Direction Inbound -Protocol TCP -LocalPort 22 -Action Allow -RemoteAddress 100.64.0.0/10
   Get-Service sshd
   ```
   Expected: `Get-Service sshd` shows `Running`.
5. **Mac: Remote Login on.** System Settings > General > Sharing > Remote Login = on.
6. **Install wooloo-win on the PC.** Download the installer (`.exe`) from the latest release at github.com/RishabhChitale/wooloo-win/releases. Run it. SmartScreen warns (unsigned): click **More info**, then **Run anyway**. Launch the app.
   Expected: the app opens with an empty machine list.
7. **Pair device (code exchange).**
   a. PC app: Pair Device. Set the target to `<windows-user>@<pc-tailscale-name>`. Copy the pairing code (`wooloo-pair-v1:...`).
   b. Mac wooloo: Connections > Pair Device. Set the target to `<mac-user>@<mac-tailscale-name>`. Copy its code.
   c. Paste each code into the other app. Check the two fingerprints match on both screens, then confirm. If the PC app is not elevated and says it cannot write `administrators_authorized_keys`, run the line it shows in Admin PowerShell.
   Expected: both apps report the peer paired.
8. **Mac -> PC profile.** The Mac's pairing runs `herdr machine add wooloo-peer-<id> ...` itself. If it says "profile written; Herdr couldn't add it", that is the known risk.
   Fallback: in the Mac wooloo app, add the PC as a machine by hand using the SSH target `<windows-user>@<pc-tailscale-name>` (the `wooloo-peer-<id>` profile is already in `~/.ssh/wooloo_peers/`). Make sure `~/.ssh/config` starts with `Include ~/.ssh/wooloo_peers/*.conf`. Also try `ssh wooloo-peer-<id>` on the Mac (expected: `wooloo-peer-bridge: ...` refusal, which still proves SSH works). Report the result.

## Checks (expected result in bold)

9. **PC shows Mac's Spaces.** In the PC app, the Mac appears and its Spaces are listed. **Live: create or rename a Space on the Mac and it changes on the PC within ~2s.**
10. **Mac shows PC's Spaces.** In Mac wooloo, the PC appears with its Spaces. **Create a Space on the PC; it shows on the Mac.**
11. **Same Space on both.** Open one Space on both machines. Type `echo hello-from-pc` on the PC. **It appears on the Mac.** Type on the Mac. **It appears on the PC.**
12. **Reconnect.** On the PC quit Tailscale (tray icon > Exit, or `tailscale down`). **Both apps show the machine as disconnected.** Restore it (`tailscale up`). **Both apps reconnect within 30 seconds** with no restart. Repeat on the Mac side if time allows.

## What to send back if something fails

Say which step number failed and what you saw (screenshot is fine). Then copy the output of:

**PC (PowerShell):**
```powershell
herdr --version; tailscale status; Get-Service sshd
Get-NetFirewallRule -Name wooloo-sshd-tailscale | Format-List
Get-Content "$env:ProgramData\ssh\administrators_authorized_keys"   # keys only, no secrets
Get-WinEvent -LogName 'OpenSSH/Operational' -MaxEvents 30 | Format-List TimeCreated,Message
Get-ChildItem "$env:APPDATA\app.wooloo.win" -Recurse | Select FullName,Length
Get-Content "$env:APPDATA\app.wooloo.win\machines.toml"
```
App logs (if present) are in `%APPDATA%\app.wooloo.win` and `%LOCALAPPDATA%\app.wooloo.win`; copy any `*.log`.

**Mac (Terminal):**
```sh
herdr --version; tailscale status
cat ~/.ssh/wooloo_peers/*.conf
ssh -v wooloo-peer-<id> hello 2>&1 | tail -30
log show --last 15m --predicate 'process == "sshd"' | tail -50
ls ~/.local/state/wooloo/peer-presence/
```
Do NOT send `~/.ssh/wooloo_peer_ed25519` (the private key) or anything without `.pub` from `~/.ssh`.
