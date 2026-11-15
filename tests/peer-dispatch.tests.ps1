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
$fakeSync = Join-Path $tmp 'wooloo-sync.cmd'
Set-Content -Path $fakeSync -Encoding ASCII -Value @('@echo off', 'echo SYNC:%*', 'exit /b 0')
$fails = 0
# Shell binary under test: 'powershell' (Windows PowerShell 5.1, sshd's default shell; must pass) or 'pwsh'. Set WOOLOO_TEST_SHELL.
$sh = if ($env:WOOLOO_TEST_SHELL) { $env:WOOLOO_TEST_SHELL } else { 'powershell' }
$al = Get-Content -Raw -LiteralPath (Join-Path $PSScriptRoot '..\docs\peer-allowlist.json') | ConvertFrom-Json
$MARKER = $al.marker; $PREFIX = $al.markerPrefix
$fwd = $fake -replace '\\', '/'
$inFile = Join-Path $tmp 'in.txt'
function Run([string]$req, [string]$peer = 'studio-mac', [string]$stdin = '') {
    $env:SSH_ORIGINAL_COMMAND = $req
    [IO.File]::WriteAllText($inFile, $stdin, (New-Object Text.UTF8Encoding $false))
    $o = Join-Path $tmp 'out.txt'; $e = Join-Path $tmp 'err.txt'
    $p = Start-Process $sh -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $disp, '-Peer', ('"' + $peer + '"'), '-Herdr', $fake, '-Sync', $fakeSync) `
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
    'wooloo-sync' = 'SYNC:exchange'
    'herdr-path-probe' = 'herdr.cmd'
    'herdr-client-bridge' = 'ARGS:--session wooloo remote-client-bridge'
    'herdr-client-bridge-idle' = 'ARGS:--session wooloo remote-client-bridge --idle-timeout-v1'
    'herdr-api-bridge' = 'ARGS:--session wooloo remote-api-bridge'
    'herdr-api-check' = 'herdr-api-bridge-v1'
    'herdr-api-discovery' = 'herdr.cmd'
    'herdr-cached-api' = 'ARGS:--session wooloo remote-api-bridge'
    'sh-uname' = "Linux`nx86_64"
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

# Linux form of the candidate scripts (Herdr on a "Linux" host swaps the Homebrew lines for linuxbrew): allowed, PC-only
$macBrewT = "    emit `"/opt/homebrew/bin/herdr`"`n    emit `"/usr/local/bin/herdr`"`n"
$linBrewT = "    emit `"/home/linuxbrew/.linuxbrew/bin/herdr`"`n"
$linCand = Fill ([string]$byName['sh-candidates'].template).Replace($macBrewT, $linBrewT) 'wooloo' $fwd
$r = Run '/bin/sh -s' 'studio-mac' ($PREFIX + $linCand)
Check 'allow Linux candidate script (stdin)' (($r.Code -eq 0) -and ($r.Out -like '*herdr.cmd*'))

# POSIX-style answers: path probes return /c/... (absolute for a POSIX client) and that form is accepted as {herdr}
$adv = '/' + $fwd.Substring(0, 1).ToLowerInvariant() + $fwd.Substring(2)
$r = Req $byName['sh-path-probe']
Check 'path probe advertises POSIX-style absolute path' (($r.Code -eq 0) -and ($r.Out -like "*`n$adv`n*"))
$r = Req $byName['sh-uname']
Check 'uname answers Linux then x86_64 after the marker' (($r.Code -eq 0) -and ($r.Out -like "*$MARKER*Linux*x86_64*") -and -not ($r.Out -like '*Windows_NT*'))
$r = Req $byName['sh-saved-bridge'] 'wooloo' $adv
Check 'allow advertised POSIX-style herdr path' (($r.Code -eq 0) -and ($r.Out -like '*ARGS:--session wooloo remote-client-bridge*'))
$r = Req $byName['sh-session-list'] 'wooloo' $adv
Check 'session list via advertised path' (($r.Code -eq 0) -and ($r.Out -like '*ARGS:session list --json*'))
$r = Req $byName['sh-exists'] 'wooloo' $adv
Check 'sh-exists via advertised path' (($r.Code -eq 0) -and ($r.Out -like "*$MARKER*"))
$r = Req $byName['sh-saved-bridge'] 'default' $adv
Check 'default session via advertised path' (($r.Code -eq 0) -and ($r.Out -like '*ARGS:remote-client-bridge*') -and -not ($r.Out -like '*--session*'))

# api probe failing: discovery exits 2, cached exits 78
$env:FAKE_NOCHECK = '1'
$r = Req $byName['herdr-api-discovery']
Check 'discovery without api support: exit 2' (($r.Code -eq 2) -and ($r.Err -like '*does not support machine API forwarding*'))
$r = Req $byName['herdr-cached-api']
Check 'cached api stale: exit 78' (($r.Code -eq 78) -and ($r.Err -like '*herdr-machine-metadata-stale-v1*'))
Remove-Item Env:FAKE_NOCHECK

# wooloo-sync takes no arguments: near misses are refused
Denied 'sync with argument' 'wooloo-peer sync x'
Denied 'sync trailing space' 'wooloo-peer sync '
Denied 'sync uppercase' 'wooloo-peer SYNC'
Denied 'sync chained' 'wooloo-peer sync; id'

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

# --- sync caps and streaming (wooloo-peer sync -> wooloo-sync exchange) ---
# Fake wooloo-sync that echoes every stdin line at once (ping-pong), so a buffering dispatcher would deadlock.
$echoSync = Join-Path $tmp 'echo-sync.ps1'
Set-Content -Path $echoSync -Encoding ASCII -Value @(
    '$in = [Console]::In',
    'while ($null -ne ($l = $in.ReadLine())) { [Console]::Out.WriteLine("ECHO:" + $l); [Console]::Out.Flush() }'
)
$echoCmd = Join-Path $tmp 'echo-sync.cmd'
Set-Content -Path $echoCmd -Encoding ASCII -Value @('@echo off', ($sh + ' -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $echoSync + '"'))
function StartSync([string]$syncExe) {
    $env:SSH_ORIGINAL_COMMAND = 'wooloo-peer sync'
    $psi = New-Object Diagnostics.ProcessStartInfo
    $psi.FileName = $sh
    $psi.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $disp + '" -Peer "studio-mac" -Herdr "' + $fake + '" -Sync "' + $syncExe + '"'
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true; $psi.RedirectStandardOutput = $true; $psi.RedirectStandardError = $true
    # No BOM: a UTF-8 preamble from the child's stdin StreamWriter would add 3 bytes to the first line (seen in CI at
    # the cap boundary). If the runtime lacks the property, say so loudly (the boundary tests do not depend on it).
    try { $psi.StandardInputEncoding = New-Object Text.UTF8Encoding($false) }
    catch { Write-Host 'harness: StandardInputEncoding unavailable' }
    [Diagnostics.Process]::Start($psi)
}
# lines are delivered one at a time while stdin stays open
$p = StartSync $echoCmd
$p.StandardInput.Write("one`n"); $p.StandardInput.Flush()
$t1 = $p.StandardOutput.ReadLineAsync()
$got1 = $t1.Wait(15000) -and ($t1.Result -eq 'ECHO:one')
Check 'sync: first line answered with stdin still open' $got1
$p.StandardInput.Write("two`n"); $p.StandardInput.Flush()
$t2 = $p.StandardOutput.ReadLineAsync()
$got2 = $t2.Wait(15000) -and ($t2.Result -eq 'ECHO:two')
Check 'sync: second line answered (ping-pong, no deadlock)' $got2
$p.StandardInput.Close()
$done = $p.WaitForExit(15000)
Check 'sync: normal passthrough exits 0 after stdin closes' ($done -and $p.ExitCode -eq 0)
if (-not $done) { try { $p.Kill() } catch {} }
# Cap-boundary cases: payload bytes go to a temp file and are fed with -RedirectStandardInput, so no StreamWriter (and
# no BOM) can touch them. Always used for these cases; the harness also fails loudly if the BOM-free encoding is missing.
function RunRawSync([byte[]]$payload) {
    $env:SSH_ORIGINAL_COMMAND = 'wooloo-peer sync'
    $raw = Join-Path $tmp 'sync-raw.bin'; $o = Join-Path $tmp 'sync-out.txt'; $e = Join-Path $tmp 'sync-err.txt'
    [IO.File]::WriteAllBytes($raw, $payload)
    $p = Start-Process $sh -ArgumentList @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $disp, '-Peer', '"studio-mac"', '-Herdr', $fake, '-Sync', $echoCmd) `
        -PassThru -NoNewWindow -RedirectStandardInput $raw -RedirectStandardOutput $o -RedirectStandardError $e
    # Windows PowerShell 5.1 quirk: ExitCode reads empty unless the process handle was opened before it exited.
    $null = $p.Handle
    $done = $p.WaitForExit(60000)
    if ($done) { $p.WaitForExit() }   # flush the async stream state before reading ExitCode and the output files
    if (-not $done) { try { $p.Kill() } catch {} }
    [pscustomobject]@{ Done = $done; Code = $(if ($done) { $p.ExitCode } else { -1 }); Out = (Get-Content -Raw -LiteralPath $o); Err = (Get-Content -Raw -LiteralPath $e) }
}
# a line over 1 MiB (1,048,577 bytes + LF): exit 65 + message
$raw2 = New-Object byte[] 1048578
for ($k = 0; $k -lt 1048577; $k++) { $raw2[$k] = 120 }
$raw2[1048577] = 10
$r = RunRawSync $raw2
Check 'sync: line over 1 MiB -> exit 65' ($r.Done -and $r.Code -eq 65)
Check 'sync: line over 1 MiB -> message' ($r.Err -like '*wooloo-peer-bridge: sync input over the size limit*')
# a line of exactly 1,048,576 bytes (newline not counted) is at the cap and still passes
# Mac rule (awk, LC_ALL=C): length of the line WITHOUT its newline, in bytes, > 1048576 is refused; ASCII so chars == bytes.
$raw = New-Object byte[] 1048577
for ($k = 0; $k -lt 1048576; $k++) { $raw[$k] = 121 }
$raw[1048576] = 10
$r = RunRawSync $raw
# Output is 'ECHO:' (5) + the line (1,048,576) + CRLF from the fake child's Write-Output (2) = 1,048,583 chars.
$okOut = ($null -ne $r.Out) -and ($r.Out.Length -ge 1048576 + 5) -and $r.Out.StartsWith('ECHO:yyyy')
Check 'sync: line at the cap passes' ($r.Done -and $r.Code -eq 0 -and $okOut)
if (-not ($r.Done -and $r.Code -eq 0 -and $okOut)) { Write-Host "     exit $($r.Code) done=$($r.Done) outlen=$($r.Out.Length) err: $($r.Err)" }
# total over 32 MiB made of many small lines: exit 65, earlier lines stay delivered
$p = StartSync $echoCmd
$errT = $p.StandardError.ReadToEndAsync()
$outT = $p.StandardOutput.ReadToEndAsync()
$chunk = ('z' * 1048000) + "`n"
try { for ($i = 0; $i -lt 34; $i++) { $p.StandardInput.Write($chunk) }; $p.StandardInput.Flush() } catch {}
$done = $p.WaitForExit(60000)
Check 'sync: total over 32 MiB -> exit 65' ($done -and $p.ExitCode -eq 65)
Check 'sync: total over 32 MiB -> message' ($errT.Wait(5000) -and ($errT.Result -like '*wooloo-peer-bridge: sync input over the size limit*'))
Check 'sync: lines before the cap stayed delivered' ($outT.Wait(5000) -and $outT.Result.StartsWith('ECHO:zzzz'))
if (-not $done) { try { $p.Kill() } catch {} }
Remove-Item Env:SSH_ORIGINAL_COMMAND -ErrorAction SilentlyContinue

Remove-Item -Recurse -Force $tmp
if ($fails -gt 0) { Write-Host "$fails failed"; exit 1 }
Write-Host 'all passed'
