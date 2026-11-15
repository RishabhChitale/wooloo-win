# Windows OpenSSH setup for wooloo peer pairing

For a PC that another of your own devices (Mac or Windows) pairs with over Tailscale. Run everything in
PowerShell as Administrator unless noted. Never expose SSH or Herdr to the open internet: the firewall rule below
only admits Tailscale addresses.

## 1. Install and start OpenSSH Server

```powershell
Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Add-WindowsCapability -Online -Name OpenSSH.Client~~~~0.0.1.0   # ssh-keygen.exe, ssh.exe
Set-Service -Name sshd -StartupType Automatic
Start-Service sshd
```

`ssh-keygen.exe` (system OpenSSH client) is what wooloo uses for the pairing key and fingerprints.
The host key wooloo advertises is `C:\ProgramData\ssh\ssh_host_ed25519_key.pub`; it exists after sshd first starts.

## 2. Firewall: Tailscale only

The installer creates an `OpenSSH-Server-In-TCP` rule open to any address. Replace it:

```powershell
Remove-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -ErrorAction SilentlyContinue
New-NetFirewallRule -Name 'wooloo-sshd-tailscale' -DisplayName 'OpenSSH (Tailscale only)' `
  -Direction Inbound -Protocol TCP -LocalPort 22 -Action Allow -RemoteAddress 100.64.0.0/10
```

## 3. Default shell

The forced command is run by `powershell`, so the default shell can stay `cmd` (the Windows default). If you changed it:

```powershell
New-ItemProperty -Path 'HKLM:\SOFTWARE\OpenSSH' -Name DefaultShell -Value 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -PropertyType String -Force
```

Either works; the command line in the key entry names `powershell` explicitly.

## 4. Which authorized_keys file applies

- Your account is in the **Administrators** group: sshd reads **only** `C:\ProgramData\ssh\administrators_authorized_keys`
  (a key in `~\.ssh\authorized_keys` is ignored). The Pair flow writes there, which needs an elevated app; if it cannot,
  it shows the line to append from an Administrator PowerShell.
- Standard user: `C:\Users\<you>\.ssh\authorized_keys`.

The file must only be writable by SYSTEM and Administrators, or sshd ignores it:

```powershell
$f = "$env:ProgramData\ssh\administrators_authorized_keys"
icacls $f /inheritance:r /grant "SYSTEM:F" /grant "Administrators:F"
```

(For `~\.ssh\authorized_keys`: `icacls $f /inheritance:r /grant "$env:USERNAME:F" /grant "SYSTEM:F"`.)

## 5. What the pairing entry looks like

```
command="powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File C:/Users/you/AppData/Roaming/app.wooloo.win/peer-dispatch.ps1 -Peer studio-mac",restrict ssh-ed25519 AAAA...
```

`restrict` turns off port/agent/X11 forwarding and PTY. The dispatcher (`resources/peer-dispatch.ps1`) never runs the
request: it matches it against a fixed list (`wooloo-peer hello|api S|client S|check S`, or the one-line Herdr bridge
`exec <path> [--session S] remote-api-bridge|remote-client-bridge [--idle-timeout-v1|--check]`), then runs the paired
`herdr.exe` itself. Anything else exits 64 with `wooloo-peer-bridge: <reason>` on stderr. Same allow-list as the Mac side.

### Sync cap and presence

`wooloo-peer sync` runs `wooloo-sync exchange` with the same caps as the Mac dispatcher: a line over 1 MiB (1,048,576
bytes) or a stream over 32 MiB (33,554,432 bytes) stops the exchange with `wooloo-peer-bridge: sync input over the size
limit` on stderr and exit 65. Caps apply to complete lines; lines already forwarded stay delivered. The exchange is
ping-pong over one open stream, so each line is forwarded and flushed immediately (never buffered to EOF), and the output
is forwarded as produced. Input is only bytes on a pipe; it is never parsed or evaluated.

Intentionally absent on Windows: the Mac dispatcher's presence files (`~/.local/state/wooloo/peer-presence/<id>.<pid>`
for bridge requests) and ready-marker framing for sync. The sync request is not marker-framed, and the Windows app does
not scan for presence yet; add both together if a Windows presence view is wanted.

## Unverified

Written against the Mac design doc without Herdr's exact preparation-probe strings or a real Windows run:
Herdr 0.9.5 request strings, multi-line `SSH_ORIGINAL_COMMAND` through Windows sshd, and the effect of `restrict`
(no PTY) on the bridges. The dispatcher follows the shared `docs/peer-allowlist.json` (20 exact templates, revision 2); `scripts/check-allowlist-drift.py` keeps its embedded copy in sync.
