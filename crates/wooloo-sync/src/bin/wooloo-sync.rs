//! `wooloo-sync`: command-line front end for the sync crate (docs/sync-protocol.md).
//!
//! - `wooloo-sync sync <ssh-target>` runs one exchange as the initiator over
//!   `ssh <target> wooloo-sync exchange`.
//! - `wooloo-sync sync --loopback` runs the responder as a local child process
//!   instead of over ssh (tests, two replicas on one machine).
//! - `wooloo-sync exchange` is the responder: it speaks the protocol on
//!   stdin/stdout and writes nothing else to them.
//! - `wooloo-sync edit <key> <field> <value>` makes one local write.
//! - `wooloo-sync node` prints this machine's node id (creating the state file
//!   if needed).

use serde_json::Value;
use std::io::{self, BufReader};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use wooloo_sync::canonical::canonical;
use wooloo_sync::state::State;
use wooloo_sync::{exchange, now_ms, state, unitcat, validate, Error, Outcome, Result};

const DEFAULT_REMOTE_CMD: &str = "wooloo-sync exchange";

/// Set by the responder: ssh forwards its stderr to the peer's terminal, and
/// local errors can carry file paths (section 9), so it writes nothing there.
static QUIET: AtomicBool = AtomicBool::new(false);

fn warn(msg: std::fmt::Arguments<'_>) {
    if !QUIET.load(Ordering::Relaxed) {
        eprintln!("wooloo-sync: {msg}");
    }
}

const USAGE: &str = "\
usage:
  wooloo-sync sync <ssh-target> [--remote-cmd CMD]
      Exchange Space metadata with a peer over `ssh -o BatchMode=yes -- <ssh-target> CMD`
      (CMD defaults to `wooloo-sync exchange`).
  wooloo-sync sync --loopback [--peer-state PATH] [--peer-unitcat PATH] [--remote-cmd CMD]
      Same exchange, but the responder runs as a local child process: this
      program's `exchange` with WOOLOO_SYNC_STATE=PATH (and WOOLOO_SYNC_UNITCAT),
      or CMD run by the shell (sh -c, or cmd /C on Windows). Needs --peer-state
      or --remote-cmd, since a replica cannot sync with itself.
  wooloo-sync edit <key> <field> <value>
  wooloo-sync edit <key> <field> --null
  wooloo-sync edit <key> <field> --json <json>
      One local write. <key> is space:<uuid> (a bare uuid also works) or
      unitcat:<space name>. For name, category, color, notes, host and herdr_id
      <value> is taken as literal text; for order, deleted and pinned_tabs it is
      JSON (3.5, true, [\"a\",\"b\"]). --null clears a field; --json forces JSON.
      Prints the stored op.
  wooloo-sync exchange
      Responder side; run by ssh on the peer. Speaks the protocol on stdin/stdout.
  wooloo-sync node
      Print this machine's node id.
  wooloo-sync help
      Show this text.

environment:
  WOOLOO_SYNC_STATE    state file (default ~/.config/wooloo-sync/state.json)
  WOOLOO_SYNC_UNITCAT  unit categories file (default ~/.local/state/unit-space-categories.json)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match argv.as_slice() {
        [] => return usage_error("no command given"),
        ["-h" | "--help" | "help"] => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        ["exchange"] => cmd_exchange(),
        ["node"] => cmd_node(),
        ["sync", rest @ ..] => match parse_sync(rest) {
            Ok(peer) => cmd_sync(&peer),
            Err(msg) => return usage_error(&msg),
        },
        ["edit", rest @ ..] => match parse_edit(rest) {
            Ok((key, field, value)) => cmd_edit(&key, field, value),
            Err(msg) => return usage_error(&msg),
        },
        _ => return usage_error("unknown command or wrong arguments"),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            warn(format_args!("{e}"));
            ExitCode::FAILURE
        }
    }
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("wooloo-sync: {msg}");
    eprintln!("{USAGE}");
    ExitCode::from(2u8)
}

/// Where the responder runs.
#[derive(Debug, PartialEq)]
enum Peer<'a> {
    Ssh {
        target: &'a str,
        remote_cmd: &'a str,
    },
    Local {
        remote_cmd: Option<&'a str>,
        state: Option<&'a str>,
        unitcat: Option<&'a str>,
    },
}

/// `<target> [--remote-cmd CMD]` or `--loopback [...]`. Unknown options are
/// refused so nothing reaches ssh as an option.
fn parse_sync<'a>(rest: &[&'a str]) -> std::result::Result<Peer<'a>, String> {
    let mut target: Option<&'a str> = None;
    let mut remote_cmd: Option<&'a str> = None;
    let mut peer_state: Option<&'a str> = None;
    let mut peer_unitcat: Option<&'a str> = None;
    let mut loopback = false;
    let mut it = rest.iter().copied();
    while let Some(a) = it.next() {
        match a {
            "--remote-cmd" | "--peer-state" | "--peer-unitcat" => {
                let v = match it.next() {
                    Some(v) if !v.trim().is_empty() => v,
                    _ => return Err(format!("{a} needs a value")),
                };
                match a {
                    "--remote-cmd" => remote_cmd = Some(v),
                    "--peer-state" => peer_state = Some(v),
                    _ => peer_unitcat = Some(v),
                }
            }
            "--loopback" => loopback = true,
            _ if a.starts_with('-') => return Err(format!("unknown option {a}")),
            _ if target.is_some() => return Err("sync takes one ssh target".into()),
            _ => target = Some(a),
        }
    }
    if loopback {
        if target.is_some() {
            return Err("--loopback takes no ssh target".into());
        }
        if remote_cmd.is_none() && peer_state.is_none() {
            return Err("--loopback needs --peer-state or --remote-cmd (a replica cannot sync with itself)".into());
        }
        return Ok(Peer::Local {
            remote_cmd,
            state: peer_state,
            unitcat: peer_unitcat,
        });
    }
    if peer_state.is_some() || peer_unitcat.is_some() {
        return Err("--peer-state and --peer-unitcat need --loopback".into());
    }
    match target {
        Some(t) => Ok(Peer::Ssh {
            target: t,
            remote_cmd: remote_cmd.unwrap_or(DEFAULT_REMOTE_CMD),
        }),
        None => Err("sync needs an ssh target (or --loopback)".into()),
    }
}

/// Fields whose CLI value is literal text; the rest take JSON.
const TEXT_FIELDS: &[&str] = &["name", "category", "color", "notes", "host", "herdr_id"];

/// `<key> <field> (<value> | --null | --json <json>)`.
fn parse_edit<'a>(rest: &[&'a str]) -> std::result::Result<(String, &'a str, Value), String> {
    let (key, field, value) = match rest {
        [key, field, "--null"] => (*key, *field, Value::Null),
        [key, field, "--json", raw] => (*key, *field, parse_json(raw)?),
        [key, field, raw] if !raw.starts_with("--") => {
            let v = if TEXT_FIELDS.contains(field) {
                Value::String((*raw).to_owned())
            } else {
                parse_json(raw)?
            };
            (*key, *field, v)
        }
        _ => return Err("edit takes <key> <field> <value> (or --null, or --json <json>)".into()),
    };
    let key = if validate::is_uuid(key) {
        format!("space:{key}")
    } else {
        key.to_owned()
    };
    if validate::key_kind(&key).is_none() {
        return Err(format!("bad key {key:?}: use space:<uuid> or unitcat:<space name>"));
    }
    Ok((key, field, value))
}

fn parse_json(raw: &str) -> std::result::Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("value is not JSON ({e}): {raw}"))
}

fn state_path() -> Result<std::path::PathBuf> {
    state::default_path()
        .ok_or_else(|| Error::Invalid("cannot find home directory (set HOME or WOOLOO_SYNC_STATE)".into()))
}

/// Section 7 import. Problems are warnings: the categories file is optional.
fn import_unitcats(s: &mut State, now: u64) {
    let Some(path) = unitcat::default_path() else { return };
    let file = unitcat::read_file(&path);
    if let Err(e) = unitcat::import(&mut s.replica, &mut s.unitcat_snapshot, file.as_ref(), now) {
        warn(format_args!("warning: unit categories not imported: {e}"));
    }
}

/// Section 7 export. Problems are warnings.
fn export_unitcats(s: &mut State) {
    let Some(path) = unitcat::default_path() else { return };
    if let Err(e) = unitcat::export(&s.replica, &mut s.unitcat_snapshot, &path) {
        warn(format_args!("warning: unit categories not exported: {e}"));
    }
}

/// Responder. stdout carries only protocol messages; stderr stays empty.
fn cmd_exchange() -> Result<()> {
    QUIET.store(true, Ordering::Relaxed);
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut input = stdin.lock();
    let mut output = stdout.lock();
    let setup = state_path().and_then(|p| state::update(&p, |s| Ok(s.replica.node.clone())).map(|n| (p, n)));
    let (path, node) = match setup {
        Ok(v) => v,
        Err(e) => {
            exchange::send_error(&mut output, &e);
            return Err(e);
        }
    };
    exchange::respond(&mut input, &mut output, &node, |_peer, ops, malformed| {
        state::update(&path, |s| {
            if s.replica.node != node {
                return Err(Error::Invalid("node id changed during the exchange".into()));
            }
            let now = now_ms();
            import_unitcats(s, now);
            let absorbed = s.replica.absorb(ops, malformed, now);
            export_unitcats(s);
            Ok(absorbed)
        })
    })?;
    Ok(())
}

/// A command run by the platform shell (loopback `--remote-cmd`).
fn shell(cmd: &str) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    }
}

fn peer_command(peer: &Peer<'_>) -> Result<Command> {
    match peer {
        Peer::Ssh { target, remote_cmd } => {
            let mut c = Command::new("ssh");
            c.arg("-o").arg("BatchMode=yes").arg("--").arg(target).arg(remote_cmd);
            Ok(c)
        }
        Peer::Local {
            remote_cmd,
            state,
            unitcat,
        } => {
            let mut c = match remote_cmd {
                Some(cmd) => shell(cmd),
                None => {
                    let mut c = Command::new(std::env::current_exe()?);
                    c.arg("exchange");
                    c
                }
            };
            if let Some(s) = state {
                c.env("WOOLOO_SYNC_STATE", s);
            }
            if let Some(u) = unitcat {
                c.env("WOOLOO_SYNC_UNITCAT", u);
            }
            Ok(c)
        }
    }
}

/// Initiator. The state lock is not held while the peer command runs.
fn cmd_sync(peer: &Peer<'_>) -> Result<()> {
    let path = state_path()?;
    let (node, regs) = state::update(&path, |s| {
        import_unitcats(s, now_ms());
        Ok((s.replica.node.clone(), s.replica.sorted_regs()))
    })?;

    let what = match peer {
        Peer::Ssh { .. } => "ssh",
        Peer::Local { .. } => "the loopback peer",
    };
    let mut child = peer_command(peer)?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| Error::Io(io::Error::new(e.kind(), format!("cannot start {what}: {e}"))))?;

    let (mut to_peer, from_peer) = match (child.stdin.take(), child.stdout.take()) {
        (Some(i), Some(o)) => (i, o),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Protocol(format!("{what} pipes unavailable")));
        }
    };
    let mut from_peer = BufReader::new(from_peer);
    let result = exchange::initiate(&mut from_peer, &mut to_peer, &node, &regs);
    // Close our side so the remote command sees EOF.
    drop(to_peer);
    let exchange::Initiated {
        peer: peer_node,
        reply,
        malformed,
        accepted,
        rejected: peer_rejected,
    } = match result {
        Ok(r) => r,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let received = reply.len() as u64 + malformed;

    let applied = state::update(&path, |s| {
        if s.replica.node != node {
            return Err(Error::Invalid("node id changed during the exchange".into()));
        }
        let now = now_ms();
        // Pick up category edits made while the peer ran before applying the reply.
        import_unitcats(s, now);
        let mut stored = 0u64;
        let mut rejected = malformed;
        for r in reply {
            match s.replica.apply(r, now) {
                Outcome::Stored => stored += 1,
                Outcome::Rejected(_) => rejected += 1,
                Outcome::Duplicate | Outcome::Ignored => {}
            }
        }
        export_unitcats(s);
        Ok((stored, rejected))
    });

    drop(from_peer);
    match child.wait() {
        Ok(st) if st.success() => {}
        Ok(st) => warn(format_args!("warning: {what} exited with {st}")),
        Err(e) => warn(format_args!("warning: cannot wait for {what}: {e}")),
    }
    let (stored, rejected) = applied?;
    println!(
        "synced with {peer_node}: sent {}, received {received} ({stored} new, {rejected} rejected); peer accepted {accepted}, rejected {peer_rejected}",
        regs.len()
    );
    Ok(())
}

/// One local write. Unit categories are imported first and exported after, so
/// a `unitcat:` edit reaches the categories file straight away.
fn cmd_edit(key: &str, field: &str, value: Value) -> Result<()> {
    let path = state_path()?;
    let op = state::update(&path, |s| {
        let now = now_ms();
        import_unitcats(s, now);
        let op = s.replica.set(key, field, value, now)?;
        export_unitcats(s);
        Ok(op)
    })?;
    println!("{}", canonical(&op.to_json()));
    Ok(())
}

fn cmd_node() -> Result<()> {
    let path = state_path()?;
    let node = state::update(&path, |s| Ok(s.replica.node.clone()))?;
    println!("{node}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const U: &str = "6f1c7a52-3b0e-4d8e-9a51-2a8f1c0d9e11";

    #[test]
    fn sync_args() {
        assert_eq!(
            parse_sync(&["mac"]),
            Ok(Peer::Ssh {
                target: "mac",
                remote_cmd: DEFAULT_REMOTE_CMD
            })
        );
        assert_eq!(
            parse_sync(&["mac", "--remote-cmd", "/opt/ws exchange"]),
            Ok(Peer::Ssh {
                target: "mac",
                remote_cmd: "/opt/ws exchange"
            })
        );
        assert_eq!(
            parse_sync(&["--loopback", "--peer-state", "/t/b.json"]),
            Ok(Peer::Local {
                remote_cmd: None,
                state: Some("/t/b.json"),
                unitcat: None
            })
        );
        assert!(parse_sync(&[]).is_err());
        assert!(parse_sync(&["-oProxyCommand=x"]).is_err());
        assert!(parse_sync(&["a", "b"]).is_err());
        assert!(parse_sync(&["mac", "--remote-cmd"]).is_err());
        assert!(parse_sync(&["--loopback"]).is_err());
        assert!(parse_sync(&["--loopback", "mac", "--peer-state", "x"]).is_err());
        assert!(parse_sync(&["mac", "--peer-state", "x"]).is_err());
    }

    #[test]
    fn edit_args() {
        let key = format!("space:{U}");
        assert_eq!(parse_edit(&[U, "name", "123"]), Ok((key.clone(), "name", json!("123"))));
        assert_eq!(parse_edit(&[key.as_str(), "order", "2.5"]), Ok((key.clone(), "order", json!(2.5))));
        assert_eq!(parse_edit(&[key.as_str(), "deleted", "true"]), Ok((key.clone(), "deleted", json!(true))));
        assert_eq!(parse_edit(&[key.as_str(), "notes", "--null"]), Ok((key.clone(), "notes", Value::Null)));
        assert_eq!(
            parse_edit(&[key.as_str(), "pinned_tabs", "[\"a\"]"]),
            Ok((key.clone(), "pinned_tabs", json!(["a"])))
        );
        assert_eq!(
            parse_edit(&[key.as_str(), "name", "--json", "\"x\""]),
            Ok((key.clone(), "name", json!("x")))
        );
        assert_eq!(
            parse_edit(&["unitcat:study central", "category", "school"]),
            Ok(("unitcat:study central".to_owned(), "category", json!("school")))
        );
        assert!(parse_edit(&["nope", "name", "x"]).is_err());
        assert!(parse_edit(&[key.as_str(), "order", "two"]).is_err());
        assert!(parse_edit(&[key.as_str(), "name"]).is_err());
        assert!(parse_edit(&[key.as_str(), "name", "--bogus"]).is_err());
    }
}
