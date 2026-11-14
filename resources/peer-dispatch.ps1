# wooloo peer dispatcher (Windows). Used as the forced command of a paired peer's SSH key:
#   command="powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <this file> -Peer <id>",restrict <key>
# The request in SSH_ORIGINAL_COMMAND is never executed. It is matched against a fixed allow-list; only then
# does this script run herdr with arguments it builds itself. Everything else: exit 64.
param(
    [Parameter(Mandatory = $true)][string]$Peer,
    [string]$Herdr = ''
)
$ErrorActionPreference = 'Stop'

function Deny([string]$why) {
    [Console]::Error.WriteLine("wooloo-peer-bridge: $why")
    exit 64
}

if ($Peer -notmatch '^[a-z0-9-]{1,32}$') { Deny 'bad peer name' }

$req = $env:SSH_ORIGINAL_COMMAND
if ([string]::IsNullOrEmpty($req)) { Deny 'empty request' }
if ($req.Length -gt 16384) { Deny 'request too large' }

# Herdr binary: the paired path (validated) or herdr.exe on PATH.
if ($Herdr -ne '') {
    if ($Herdr -notmatch '^[A-Za-z]:[A-Za-z0-9._ /\\()-]+$' -or $Herdr -match '\.\.') { Deny 'bad herdr path' }
    $herdrPath = $Herdr
} else {
    $cmd = Get-Command herdr.exe -ErrorAction SilentlyContinue
    if (-not $cmd) { $cmd = Get-Command herdr -ErrorAction SilentlyContinue }
    if (-not $cmd) { Deny 'herdr not found' }
    $herdrPath = $cmd.Source
}
if (-not (Test-Path -LiteralPath $herdrPath -PathType Leaf)) { Deny 'herdr not found' }

$sess = '[A-Za-z0-9_-]{1,64}'
$verb = $null      # remote-api-bridge | remote-client-bridge
$flag = $null      # --check | --idle-timeout-v1 | $null
$session = $null
$marker = $false
$hello = $false

$r = $req.TrimEnd("`r", "`n")
if ($r -ceq 'wooloo-peer hello') {
    $hello = $true
} elseif ($r -match "^wooloo-peer (api|client|check) ($sess)$") {
    $session = $Matches[2]
    switch ($Matches[1]) {
        'api'    { $verb = 'remote-api-bridge' }
        'client' { $verb = 'remote-client-bridge' }
        'check'  { $verb = 'remote-api-bridge'; $flag = '--check' }
    }
} else {
    # Herdr bridge script: exactly one line `exec <path> [--session S] <verb> [flag] [</dev/null]` (trailing ' allowed).
    $execLines = @($r -split "`r?`n" | Where-Object { $_ -match '^\s*exec\s' })
    if ($execLines.Count -ne 1) { Deny 'request not allowed' }
    # Every other line must be a harmless shell comment or blank, or the marker echo; nothing else passes.
    foreach ($l in ($r -split "`r?`n")) {
        if ($l -match '^\s*exec\s') { continue }
        if ($l -match '^\s*$' -or $l -match '^\s*#') { continue }
        if ($l -match '^\s*echo herdr-remote-output-ready:1\s*$') { $marker = $true; continue }
        Deny 'request not allowed'
    }
    $line = $execLines[0].Trim()
    $re = "^exec\s+\S+(?:\s+--session\s+($sess))?\s+(remote-api-bridge|remote-client-bridge)(?:\s+(--idle-timeout-v1|--check))?(?:\s+</dev/null)?'?$"
    if (-not ($line -match $re)) { Deny 'request not allowed' }
    if ($Matches[1]) { $session = $Matches[1] }
    $verb = $Matches[2]
    if ($Matches[3]) { $flag = $Matches[3] }
    # the request's own path is ignored; the paired herdr is used
    $stripped = $line -replace '</dev/null', ''
    if ($stripped -match '[;&|`$<>(){}\\"]') { Deny 'request not allowed' }
}

if ($hello) {
    $v = (& $herdrPath --version 2>$null | Select-Object -First 1)
    if ($LASTEXITCODE -ne 0 -or -not $v) { Deny 'herdr version failed' }
    $v = ($v -replace '^\s*herdr\s+', '').Trim()
    if ($v -notmatch '^[0-9A-Za-z.+-]{1,32}$') { Deny 'bad herdr version' }
    [Console]::Out.Write("wooloo-peer-bridge-v1 $v`n")
    exit 0
}

if ($marker) { [Console]::Out.Write("herdr-remote-output-ready:1`n"); [Console]::Out.Flush() }

$argList = @()
if ($session) { $argList += @('--session', $session) }
$argList += $verb
if ($flag) { $argList += $flag }

# Child inherits stdin/stdout/stderr, so the bridge streams straight through ssh.
$p = Start-Process -FilePath $herdrPath -ArgumentList $argList -NoNewWindow -Wait -PassThru
exit $p.ExitCode
