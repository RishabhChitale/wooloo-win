# wooloo peer dispatcher (Windows). Used as the forced command of a paired peer's SSH key:
#   command="powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File <this file> -Peer <id>",restrict <key>
# The request is never executed. It must equal one allow-list template exactly (no globbing, no trimming) after
# the slots are filled; only then does this script run the paired herdr with arguments it builds itself.
# Everything else: "wooloo-peer-bridge: <reason>" on stderr, exit 64.
# Templates: docs/peer-allowlist.json (shared with the Mac side), embedded below by scripts/check-allowlist-drift.py.
param(
    [Parameter(Mandatory = $true)][string]$Peer,
    [string]$Herdr = '',
    [string]$Sync = ''
)
$ErrorActionPreference = 'Stop'

function Deny([string]$why) {
    [Console]::Error.WriteLine("wooloo-peer-bridge: $why")
    exit 64
}

# BEGIN ALLOWLIST (generated; do not edit by hand)
$alJson = @'
{"marker":"herdr-remote-output-ready:1","markerPrefix":"printf '\n%s\n' 'herdr-remote-output-ready:1'\n","entries":[{"name":"wooloo-hello","transport":"argv","template":"wooloo-peer hello"},{"name":"wooloo-api","transport":"argv","template":"wooloo-peer api {session}"},{"name":"wooloo-client","transport":"argv","template":"wooloo-peer client {session}"},{"name":"wooloo-check","transport":"argv","template":"wooloo-peer check {session}"},{"name":"wooloo-sync","transport":"argv","template":"wooloo-peer sync"},{"name":"herdr-path-probe","transport":"argv","template":"printf '\n%s\n' 'herdr-remote-output-ready:1'\ncommand -v herdr"},{"name":"herdr-client-bridge","transport":"argv","template":"printf '\n%s\n' 'herdr-remote-output-ready:1'\nexec {herdr} {sessionArgs}remote-client-bridge"},{"name":"herdr-client-bridge-idle","transport":"argv","template":"printf '\n%s\n' 'herdr-remote-output-ready:1'\nexec {herdr} {sessionArgs}remote-client-bridge --idle-timeout-v1"},{"name":"herdr-api-bridge","transport":"argv","template":"printf '\n%s\n' 'herdr-remote-output-ready:1'\nexec {herdr} --session {session} remote-api-bridge"},{"name":"herdr-api-check","transport":"argv","template":"printf '\n%s\n' 'herdr-remote-output-ready:1'\nexec {herdr} --session {session} remote-api-bridge --check"},{"name":"herdr-api-discovery","transport":"argv","template":"/bin/sh -c 'printf '\\''\n%s\n'\\'' '\\''herdr-remote-output-ready:1'\\''\nset -f\ncandidates=$(\ncommand -v herdr\nhome=${HOME:-}\nuser=${USER:-}\nversion={version}\nemit() {\n    path=$1\n    if [ -n \"$path\" ] && [ -x \"$path\" ]; then\n        printf '\\''%s\\n'\\'' \"$path\"\n    fi\n}\nif [ -n \"$home\" ]; then\n    emit \"$home/.local/bin/herdr\"\nfi\n    emit \"/opt/homebrew/bin/herdr\"\n    emit \"/usr/local/bin/herdr\"\nif [ -n \"$home\" ]; then\n    emit \"$home/.local/share/mise/installs/herdr/$version/bin/herdr\"\n    emit \"$home/.local/share/mise/installs/herdr/$version/herdr\"\n    emit \"$home/.local/share/mise/installs/github-ogulcancelik-herdr/$version/herdr\"\n    emit \"$home/.nix-profile/bin/herdr\"\nfi\nif [ -n \"$user\" ]; then\n    emit \"/etc/profiles/per-user/$user/bin/herdr\"\nfi\nemit \"/nix/var/nix/profiles/default/bin/herdr\"\nemit \"/run/current-system/sw/bin/herdr\"\n\n)\nIFS='\\''\n'\\''\nfor candidate in $candidates; do\n    case \"$candidate\" in\n        */mise/shims/herdr) continue ;;\n        /*) ;;\n        *) continue ;;\n    esac\n    [ -x \"$candidate\" ] || continue\n    if capability=$(\"$candidate\" --session {session} remote-api-bridge --check </dev/null 2>/dev/null) && [ \"$capability\" = herdr-api-bridge-v1 ]; then\n        printf '\\''%s\\n'\\'' \"$candidate\"\n        exit 0\n    fi\ndone\nprintf '\\''%s\\n'\\'' '\\''remote Herdr does not support machine API forwarding; update Herdr on this machine'\\'' >&2\nexit 2'"},{"name":"herdr-cached-api","transport":"argv","template":"/bin/sh -c 'if capability=$({herdr} --session {session} remote-api-bridge --check </dev/null 2>/dev/null) && [ \"$capability\" = herdr-api-bridge-v1 ]; then\nprintf '\\''\n%s\n'\\'' '\\''herdr-remote-output-ready:1'\\''\nexec {herdr} --session {session} remote-api-bridge\nelse\n    printf '\\''%s\\n'\\'' '\\''herdr-machine-metadata-stale-v1'\\'' >&2\n    exit 78\nfi'"},{"name":"sh-uname","transport":"stdin","template":"uname -s\nuname -m\n"},{"name":"sh-path-probe","transport":"stdin","template":"command -v herdr\n"},{"name":"sh-candidates","transport":"stdin","template":"home=${HOME:-}\nuser=${USER:-}\nversion={version}\nemit() {\n    path=$1\n    if [ -n \"$path\" ] && [ -x \"$path\" ]; then\n        printf '%s\\n' \"$path\"\n    fi\n}\nif [ -n \"$home\" ]; then\n    emit \"$home/.local/bin/herdr\"\nfi\n    emit \"/opt/homebrew/bin/herdr\"\n    emit \"/usr/local/bin/herdr\"\nif [ -n \"$home\" ]; then\n    emit \"$home/.local/share/mise/installs/herdr/$version/bin/herdr\"\n    emit \"$home/.local/share/mise/installs/herdr/$version/herdr\"\n    emit \"$home/.local/share/mise/installs/github-ogulcancelik-herdr/$version/herdr\"\n    emit \"$home/.nix-profile/bin/herdr\"\nfi\nif [ -n \"$user\" ]; then\n    emit \"/etc/profiles/per-user/$user/bin/herdr\"\nfi\nemit \"/nix/var/nix/profiles/default/bin/herdr\"\nemit \"/run/current-system/sw/bin/herdr\"\n"},{"name":"sh-exists","transport":"stdin","template":"test -x {herdr}"},{"name":"sh-client-status","transport":"stdin","template":"test -x {herdr} && {herdr} status client --json"},{"name":"sh-server-status","transport":"stdin","template":"{herdr} {sessionArgs}status server --json"},{"name":"sh-session-list","transport":"stdin","template":"{herdr} session list --json"},{"name":"sh-saved-bridge","transport":"stdin","template":"exec {herdr} {sessionArgs}remote-client-bridge </dev/null"}]}
'@
# END ALLOWLIST

if ($Peer -cnotmatch '^[a-z0-9-]{1,32}$') { Deny 'bad peer name' }

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


$al = $alJson | ConvertFrom-Json
$MARKER = [string]$al.marker
$PREFIX = [string]$al.markerPrefix
$sess = '[A-Za-z0-9_-]{1,64}'

# Herdr treats this host as Linux (see the uname answer below), and its Linux candidate script differs from the macOS
# one only by swapping the two Homebrew lines for the linuxbrew line. The shared allow-list carries the macOS form, so
# derive the Linux form of the two candidate-scanning entries here (PC-only; the shared list stays unchanged).
$macBrew = "    emit `"/opt/homebrew/bin/herdr`"`n    emit `"/usr/local/bin/herdr`"`n"
$linBrew = "    emit `"/home/linuxbrew/.linuxbrew/bin/herdr`"`n"
$extra = @()
foreach ($e in @($al.entries)) {
    if ($e.name -in 'sh-candidates', 'herdr-api-discovery' -and ([string]$e.template).Contains($macBrew)) {
        $c = $e.PSObject.Copy()
        $c.template = ([string]$e.template).Replace($macBrew, $linBrew)
        $extra += $c
    }
}
$al.entries = @($al.entries) + $extra
$paired = $herdrPath -replace '\\', '/'
# Herdr treats this host as POSIX (uname answers Linux), so it expects an absolute path starting with '/': advertise
# C:/Users/x/herdr.exe as /c/Users/x/herdr.exe. The {herdr} slot accepts the advertised form and the plain paired path
# (both name the same paired binary; nothing else matches).
$advertised = if ($paired -match '^([A-Za-z]):(/.*)$') { '/' + $Matches[1].ToLowerInvariant() + $Matches[2] } else { $paired }
$pairedAlt = '(?:' + (@($advertised, $paired | Select-Object -Unique | ForEach-Object { [regex]::Escape($_) + '|' + [regex]::Escape("'" + $_ + "'") }) -join '|') + ')'

function ToRegex([string]$t) {
    $seen = @{}
    $sb = ''
    foreach ($p in [regex]::Split($t, '(\{[A-Za-z]+\})')) {
        if ($p -match '^\{([A-Za-z]+)\}$') {
            $n = $Matches[1]
            if ($seen.ContainsKey($n)) { $sb += "\k<$n>"; continue }
            $seen[$n] = $true
            switch ($n) {
                'session'     { $sb += "(?<session>$sess)" }
                'version'     { $sb += '(?<version>[0-9A-Za-z.+-]{1,32})' }
                'herdr'       { $sb += "(?<herdr>$pairedAlt)" }
                # empty for the default session, else '--session S ' (one trailing space)
                'sessionArgs' { $sb += "(?<sessionArgs>(?:--session (?!default )(?<session>$sess) )?)"; $seen['session'] = $true }
                default       { Deny 'bad template' }
            }
        } else {
            $sb += [regex]::Escape($p)
        }
    }
    return '\A' + $sb + '\z'
}

function Find-Entry([string]$transport, [string]$text) {
    foreach ($e in @($al.entries)) {
        if ($e.transport -ne $transport) { continue }
        if ($text -cmatch (ToRegex ([string]$e.template))) { $script:M = $Matches; return $e }
    }
    return $null
}

$entry = $null
if ($req -ceq '/bin/sh -s') {
    $in = [Console]::OpenStandardInput()
    $buf = New-Object byte[] 16385
    $n = 0
    while ($n -lt $buf.Length) {
        $k = $in.Read($buf, $n, $buf.Length - $n)
        if ($k -le 0) { break }
        $n += $k
    }
    if ($n -gt 16384) { Deny 'request too large' }
    $body = [Text.Encoding]::UTF8.GetString($buf, 0, $n)
    if (-not $body.StartsWith($PREFIX, [StringComparison]::Ordinal)) { Deny 'request not allowed' }
    $entry = Find-Entry 'stdin' $body.Substring($PREFIX.Length)
} else {
    # Herdr's ssh invocation appends one trailing space to the remote command; the match itself stays exact.
    $entry = Find-Entry 'argv' ($req -replace ' \z', '')
}
if (-not $entry) { Deny 'request not allowed' }
$session = [string]$script:M['session']
$sa = @()
if ($session) { $sa = @('--session', $session) }

function Marker { [Console]::Out.Write("`n$MARKER`n"); [Console]::Out.Flush() }
function Say([string]$s) { [Console]::Out.Write($s); [Console]::Out.Flush() }

# Runs the paired herdr; the child inherits stdout/stderr (and stdin unless a file is given).
function Run-Herdr([string[]]$a, [string]$stdinFile = '') {
    $sp = @{ FilePath = $herdrPath; NoNewWindow = $true; Wait = $true; PassThru = $true }
    if ($a.Count -gt 0) { $sp.ArgumentList = $a }
    if ($stdinFile) { $sp.RedirectStandardInput = $stdinFile }
    return (Start-Process @sp).ExitCode
}

function Run-Herdr-Exe([string]$exe, [string[]]$a) {
    $sp = @{ FilePath = $exe; NoNewWindow = $true; Wait = $true; PassThru = $true; ArgumentList = $a }
    return (Start-Process @sp).ExitCode
}

# `herdr --session S remote-api-bridge --check </dev/null` must print herdr-api-bridge-v1.
function Api-Capable {
    $i = [IO.Path]::GetTempFileName(); $o = [IO.Path]::GetTempFileName(); $e = [IO.Path]::GetTempFileName()
    try {
        $p = Start-Process -FilePath $herdrPath -ArgumentList ($sa + @('remote-api-bridge', '--check')) -NoNewWindow -Wait -PassThru `
            -RedirectStandardInput $i -RedirectStandardOutput $o -RedirectStandardError $e
        return ($p.ExitCode -eq 0 -and ((Get-Content -LiteralPath $o -Raw) -as [string]).Trim() -ceq 'herdr-api-bridge-v1')
    } finally {
        Remove-Item -LiteralPath $i, $o, $e -Force -ErrorAction SilentlyContinue
    }
}

switch ($entry.name) {
    'wooloo-hello' {
        # Collect all output first: `| Select-Object -First 1` stops the pipeline early and pwsh 7 then leaves $LASTEXITCODE unset/non-zero.
        $vOut = @(& $herdrPath --version 2>$null)
        $vCode = $LASTEXITCODE
        $v = $vOut | Select-Object -First 1
        if ($vCode -ne 0 -or -not $v) { Deny 'herdr version failed' }
        $v = ($v -replace '^\s*herdr\s+', '').Trim()
        if ($v -notmatch '^[0-9A-Za-z.+-]{1,32}$') { Deny 'bad herdr version' }
        Say "wooloo-peer-bridge-v1 $v`n"
        exit 0
    }
    'wooloo-api'    {
        # Session 'herdr-app' = the PC's Herdr app + herdr-core (adapter). Every other session stays on the standalone Herdr server
        # that wooloo's client bridge needs.
        if ($session -ne 'herdr-app') { exit (Run-Herdr ($sa + @('remote-api-bridge'))) }
        $py = Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312\python.exe'
        $compat = Join-Path $PSScriptRoot 'herdr_compat.py'
        if ((Test-Path -LiteralPath $py) -and (Test-Path -LiteralPath $compat)) { exit (Run-Herdr-Exe $py @('-X', 'utf8', $compat)) }
        exit (Run-Herdr ($sa + @('remote-api-bridge')))
    }
    'wooloo-client' {
        # Session 'herdr-app': text-mirror client endpoint for the app's terminals (herdr_client.py).
        if ($session -eq 'herdr-app') {
            $py = Join-Path $env:LOCALAPPDATA 'Programs\Python\Python312\python.exe'
            $cl = Join-Path $PSScriptRoot 'herdr_client.py'
            if ((Test-Path -LiteralPath $py) -and (Test-Path -LiteralPath $cl)) { exit (Run-Herdr-Exe $py @('-X', 'utf8', $cl)) }
        }
        exit (Run-Herdr ($sa + @('remote-client-bridge')))
    }
    'wooloo-sync' {
        # Metadata sync responder: no arguments, stdin/stdout are the exchange (docs/sync-protocol.md).
        if ($Sync -ne '') {
            if ($Sync -notmatch '^[A-Za-z]:[A-Za-z0-9._ /\\()-]+$' -or $Sync -match '\.\.') { Deny 'bad sync path' }
            $syncPath = $Sync
        } else {
            # shipped next to this script by the app (copied there at pairing and at every app start), else on PATH
            $syncPath = Join-Path $PSScriptRoot 'wooloo-sync.exe'
            if (-not (Test-Path -LiteralPath $syncPath -PathType Leaf)) {
                $sc = Get-Command wooloo-sync.exe -ErrorAction SilentlyContinue
                if (-not $sc) { Deny 'wooloo-sync not found' }
                $syncPath = $sc.Source
            }
        }
        if (-not (Test-Path -LiteralPath $syncPath -PathType Leaf)) { Deny 'wooloo-sync not found' }
        # Inherit stdio like Run-Herdr. Reading stdin in PowerShell (.NET console stream) over Windows sshd delivers only the
        # first chunk, so a ping-pong exchange stalls; wooloo-sync enforces the line and record caps itself.
        exit (Run-Herdr-Exe $syncPath @('exchange'))
    }
    'wooloo-check'  { exit (Run-Herdr ($sa + @('remote-api-bridge', '--check'))) }
    { $_ -in 'herdr-path-probe', 'sh-path-probe', 'sh-candidates' } { Marker; Say "$advertised`n"; exit 0 }
    'herdr-client-bridge'      { Marker; exit (Run-Herdr ($sa + @('remote-client-bridge'))) }
    'herdr-client-bridge-idle' { Marker; exit (Run-Herdr ($sa + @('remote-client-bridge', '--idle-timeout-v1'))) }
    'herdr-api-bridge'         { Marker; exit (Run-Herdr ($sa + @('remote-api-bridge'))) }
    'herdr-api-check'          { Marker; exit (Run-Herdr ($sa + @('remote-api-bridge', '--check'))) }
    'herdr-api-discovery' {
        Marker
        if (Api-Capable) { Say "$advertised`n"; exit 0 }
        [Console]::Error.WriteLine('remote Herdr does not support machine API forwarding; update Herdr on this machine')
        exit 2
    }
    'herdr-cached-api' {
        if (Api-Capable) { Marker; exit (Run-Herdr ($sa + @('remote-api-bridge'))) }
        [Console]::Error.WriteLine('herdr-machine-metadata-stale-v1')
        exit 78
    }
    'sh-uname' {
        Marker
        # Herdr only supports POSIX remotes: answer like Linux x86_64 (the allow-listed commands are the only thing that runs).
        Say "Linux`nx86_64`n"
        exit 0
    }
    'sh-exists'        { Marker; exit 0 }
    'sh-client-status' { Marker; exit (Run-Herdr @('status', 'client', '--json')) }
    'sh-server-status' { Marker; exit (Run-Herdr ($sa + @('status', 'server', '--json'))) }
    'sh-session-list'  { Marker; exit (Run-Herdr @('session', 'list', '--json')) }
    'sh-saved-bridge' {
        Marker
        $i = [IO.Path]::GetTempFileName()
        try { exit (Run-Herdr ($sa + @('remote-client-bridge')) $i) }
        finally { Remove-Item -LiteralPath $i -Force -ErrorAction SilentlyContinue }
    }
    default { Deny 'request not allowed' }
}
