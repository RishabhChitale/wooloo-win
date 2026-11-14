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
    'echo ARGS:%*',
    'exit /b 0'
)
$fails = 0
function Run([string]$req, [string]$peer = 'studio-mac') {
    $env:SSH_ORIGINAL_COMMAND = $req
    $o = Join-Path $tmp 'out.txt'; $e = Join-Path $tmp 'err.txt'
    $p = Start-Process powershell -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $disp, '-Peer', $peer, '-Herdr', $fake) `
        -Wait -PassThru -NoNewWindow -RedirectStandardOutput $o -RedirectStandardError $e
    [pscustomobject]@{ Code = $p.ExitCode; Out = (Get-Content $o -Raw); Err = (Get-Content $e -Raw) }
}
function Check([string]$name, [bool]$ok) {
    if ($ok) { Write-Host "ok   $name" } else { Write-Host "FAIL $name"; $script:fails++ }
}
function Allowed([string]$name, [string]$req, [string]$expectOut) {
    $r = Run $req
    Check $name (($r.Code -eq 0) -and ($r.Out -like "*$expectOut*"))
    if ($r.Code -ne 0) { Write-Host "     exit $($r.Code) stderr: $($r.Err)" }
}
function Denied([string]$name, [string]$req, [string]$peer = 'studio-mac') {
    $r = Run $req $peer
    Check $name (($r.Code -eq 64) -and ($r.Err -like 'wooloo-peer-bridge:*') -and -not ($r.Out -like '*ARGS:*'))
}

# allowed
Allowed 'hello' 'wooloo-peer hello' 'wooloo-peer-bridge-v1 0.9.5'
Allowed 'short api' 'wooloo-peer api wooloo' 'ARGS:--session wooloo remote-api-bridge'
Allowed 'short client' 'wooloo-peer client my_s-1' 'ARGS:--session my_s-1 remote-client-bridge'
Allowed 'short check' 'wooloo-peer check wooloo' 'ARGS:--session wooloo remote-api-bridge --check'
Allowed 'script line' "exec /usr/bin/herdr --session wooloo remote-api-bridge" 'ARGS:--session wooloo remote-api-bridge'
Allowed 'script line, no session' "exec herdr remote-client-bridge" 'ARGS:remote-client-bridge'
Allowed 'script line with </dev/null' "exec herdr --session wooloo remote-api-bridge </dev/null" 'ARGS:--session wooloo remote-api-bridge'
Allowed 'script line idle flag' "exec herdr --session wooloo remote-client-bridge --idle-timeout-v1" 'ARGS:--session wooloo remote-client-bridge --idle-timeout-v1'
Allowed 'script line trailing quote' "exec herdr --session wooloo remote-api-bridge --check'" 'ARGS:--session wooloo remote-api-bridge --check'
Allowed 'script with marker' "echo herdr-remote-output-ready:1`nexec herdr --session wooloo remote-api-bridge" 'herdr-remote-output-ready:1'

# the request's own path is ignored: the paired fake herdr ran (it prints ARGS), not C:\evil
Allowed 'request path ignored' 'exec /opt/evil/herdr --session wooloo remote-api-bridge' 'ARGS:'
Denied 'backslash path' 'exec C:\evil\herdr.exe --session wooloo remote-api-bridge'

# denied
Denied 'empty' ''
Denied 'semicolon' 'wooloo-peer hello; calc'
Denied 'semicolon in session' 'wooloo-peer api wooloo;calc'
Denied 'subexpression' 'wooloo-peer api $(calc)'
Denied 'powershell subexpression' 'wooloo-peer api wooloo$(calc)'
Denied 'backtick' 'wooloo-peer api woo`loo'
Denied 'pipe' 'wooloo-peer hello | calc'
Denied 'ampersand' 'wooloo-peer hello & calc'
Denied 'redirect' 'wooloo-peer hello > x.txt'
Denied 'two exec lines' "exec herdr remote-api-bridge`nexec herdr remote-client-bridge"
Denied 'exec plus other command' "calc`nexec herdr remote-api-bridge"
Denied 'exec with chained cmd' "exec herdr remote-api-bridge; calc"
Denied 'exec with extra arg' "exec herdr remote-api-bridge --evil"
Denied 'exec other verb' "exec herdr kill-server"
Denied 'cmd.exe /c' 'cmd.exe /c calc'
Denied 'powershell -Command' 'powershell -Command calc'
Denied 'sh -s' '/bin/sh -s'
Denied 'newline after hello' "wooloo-peer hello`ncalc"
Denied 'bad session chars' 'wooloo-peer api wo oloo'
Denied 'session too long' ("wooloo-peer api " + ('a' * 65))
Denied 'bad peer name' 'wooloo-peer hello' 'Bad Peer'
Denied 'peer with semicolon' 'wooloo-peer hello' 'a;b'
Denied 'oversize request' ('x' * 17000)

Remove-Item -Recurse -Force $tmp
if ($fails -gt 0) { Write-Host "$fails failed"; exit 1 }
Write-Host 'all passed'
