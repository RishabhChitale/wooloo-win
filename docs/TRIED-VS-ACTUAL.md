# Local Herdr endpoint: what wooloo-win tries vs what Herdr actually uses

Source: Herdr v0.9.5 (github.com/herdrdev/herdr). Items marked UNVERIFIED were read from source only and not confirmed on Windows hardware.

## Checklist

| # | Topic | Herdr actually | wooloo-win tries | Check |
|---|-------|----------------|------------------|-------|
| 1 | Default session | The unnamed root: `<config>\herdr.sock` | rc4 with no session: root path. A session of `"default"` used to become `sessions\default` (wrong). Now normalised to the root | `herdr session list` shows which one is `default` |
| 2 | Named session | `<config>\sessions\<name>\herdr.sock`; chosen by `--session` or `HERDR_SESSION` | machines.toml `session = "<name>"` | `$env:HERDR_SESSION` |
| 3 | Client channel | `herdr-client.sock` beside the API socket | Same name, derived from the API path | pipe list (below) |
| 4 | Config dir | `$XDG_CONFIG_HOME\herdr` if set, else `%APPDATA%\herdr`. Debug builds use `herdr-dev` | Same rule. A `herdr-dev` build is NOT checked | `echo $env:APPDATA $env:XDG_CONFIG_HOME` |
| 5 | Pipe name | Derived by interprocess `GenericNamespaced` from the full socket path (UNVERIFIED on hardware) | `\\.\pipe\<config>\[sessions\<s>\]herdr.sock`. `pipe_path` in machines.toml overrides | the pipe list shows the real name |
| 6 | Elevation | An elevated server can refuse a non-elevated client (integrity label) unless started with `--allow-unelevated-clients` | Connect fails with access denied | start herdr and wooloo-win at the same elevation |
| 7 | Nested Herdr | Running `herdr` inside a Herdr window prints "nested herdr is disabled by default" and does not start a server | n/a | run it from a plain PowerShell window |
| 8 | Several sessions | `herdr status server` reports the selected session only | rc4 looked at one guess | `herdr session list` |

## What rc5 does about it

For a local machine with no `session` and no `pipe_path`, every attempt:

1. runs `herdr session list --json` (hidden window, 4 s timeout);
2. if that fails, lists `\\.\pipe\` entries containing `herdr`;
3. default session running: use the root; else exactly one running: use it; else several: Failed with "found sessions: a, b; choose one" (hint: set `session`); none: the old default path, with its normal "no Herdr server found" text.

The machine's session can be set with `set_machine_session` (or `session` in machines.toml).
The dot is amber (connecting) before every attempt, including timed retries and manual reload. Retry delays are 1, 2, 4, 8, 16, 30, 30 ... s; reload resets them to 1 s.

## PowerShell: see the actual endpoint

```powershell
herdr status server
herdr session list
herdr session list --json
Get-ChildItem \\.\pipe\ | Where-Object Name -like '*herdr*'
echo $env:APPDATA $env:XDG_CONFIG_HOME $env:HERDR_SESSION
```

Compare the pipe names and the `socket_path` values from `herdr session list --json` with the "tried:" list in wooloo-win's failure text (Copy diagnostics). If the real pipe name differs from the one tried, put the real `\\.\pipe\...` path (minus the `\\.\pipe\` prefix) in machines.toml as `pipe_path`.
