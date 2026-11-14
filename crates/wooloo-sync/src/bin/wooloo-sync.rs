//! `wooloo-sync`: command-line front end for the sync crate (docs/sync-protocol.md).
//!
//! - `wooloo-sync sync <ssh-target>` runs one exchange as the initiator over
//!   `ssh <target> wooloo-sync exchange`.
//! - `wooloo-sync exchange` is the responder: it speaks the protocol on
//!   stdin/stdout and writes nothing else to them.
//! - `wooloo-sync node` prints this machine's node id (creating the state file
//!   if needed).

use std::io::{self, BufReader};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use wooloo_sync::state::State;
use wooloo_sync::{exchange, now_ms, state, unitcat, Error, Outcome, Result};

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
      Exchange Space metadata with a peer over `ssh <ssh-target> CMD`
      (CMD defaults to `wooloo-sync exchange`).
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
            Ok((target, remote_cmd)) => cmd_sync(target, remote_cmd),
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

/// `<target> [--remote-cmd CMD]`. Options other than `--remote-cmd` are
/// refused so nothing reaches ssh as an option.
fn parse_sync<'a>(rest: &[&'a str]) -> std::result::Result<(&'a str, &'a str), String> {
    let mut target: Option<&'a str> = None;
    let mut remote_cmd = DEFAULT_REMOTE_CMD;
    let mut it = rest.iter().copied();
    while let Some(a) = it.next() {
        if a == "--remote-cmd" {
            match it.next() {
                Some(c) if !c.trim().is_empty() => remote_cmd = c,
                _ => return Err("--remote-cmd needs a command".into()),
            }
        } else if a.starts_with('-') {
            return Err(format!("unknown option {a}"));
        } else if target.is_some() {
            return Err("sync takes one ssh target".into());
        } else {
            target = Some(a);
        }
    }
    match target {
        Some(t) => Ok((t, remote_cmd)),
        None => Err("sync needs an ssh target".into()),
    }
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

/// Initiator. The state lock is not held while ssh runs.
fn cmd_sync(target: &str, remote_cmd: &str) -> Result<()> {
    let path = state_path()?;
    let (node, regs) = state::update(&path, |s| {
        import_unitcats(s, now_ms());
        Ok((s.replica.node.clone(), s.replica.sorted_regs()))
    })?;

    let mut child = Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("--")
        .arg(target)
        .arg(remote_cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| Error::Io(io::Error::new(e.kind(), format!("cannot start ssh: {e}"))))?;

    let (mut to_peer, from_peer) = match (child.stdin.take(), child.stdout.take()) {
        (Some(i), Some(o)) => (i, o),
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Protocol("ssh pipes unavailable".into()));
        }
    };
    let mut from_peer = BufReader::new(from_peer);
    let result = exchange::initiate(&mut from_peer, &mut to_peer, &node, &regs);
    // Close our side so the remote command sees EOF.
    drop(to_peer);
    let exchange::Initiated {
        peer,
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
        // Pick up category edits made while ssh ran before applying the reply.
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
        Ok(st) => warn(format_args!("warning: ssh exited with {st}")),
        Err(e) => warn(format_args!("warning: cannot wait for ssh: {e}")),
    }
    let (stored, rejected) = applied?;
    println!(
        "synced with {peer}: sent {}, received {received} ({stored} new, {rejected} rejected); peer accepted {accepted}, rejected {peer_rejected}",
        regs.len()
    );
    Ok(())
}

fn cmd_node() -> Result<()> {
    let path = state_path()?;
    let node = state::update(&path, |s| Ok(s.replica.node.clone()))?;
    println!("{node}");
    Ok(())
}
