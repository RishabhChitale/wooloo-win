//! "Test connection": a read-only, hop-by-hop check of a paired/SSH machine.
//! Steps run in order and stop at the first failure: tailscale ping, TCP 22, ssh auth, bridge.
//! Pure formatting/classification lives in free functions so it can be unit tested.
use super::{s, App, Res};
use serde::Serialize;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Command as Proc, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tauri::State;
use wooloo_core::machines::MachineKind;
use wooloo_core::transport::{split_target, Channel, SshTransport};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DiagStep {
    /// "tailscale" | "route" | "tcp" | "ssh" | "bridge" | "reverse"
    pub step: String,
    pub label: String,
    /// "ok" | "warn" | "fail" | "skip"
    pub status: String,
    pub detail: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DiagReport {
    pub steps: Vec<DiagStep>,
    pub ok: bool,
    pub hint: Option<String>,
}

fn label_of(step: &str) -> &'static str {
    match step {
        "tailscale" => "Tailscale ping",
        "route" => "Route to peer",
        "tcp" => "TCP port 22",
        "ssh" => "SSH login",
        "reverse" => "Reverse: peer to this device, TCP 22",
        _ => "Herdr bridge",
    }
}

fn mk(step: &str, status: &str, detail: impl Into<String>) -> DiagStep {
    DiagStep { step: step.into(), label: label_of(step).into(), status: status.into(), detail: detail.into() }
}

pub const VPN_HINT: &str = "a VPN/TUN adapter is capturing 100.64.0.0/10; exclude it from the VPN or pause it";

/// Judges which adapter carries traffic to the peer's Tailscale IP. Pure, so it is unit-tested.
pub fn route_verdict(alias: &str) -> (&'static str, String) {
    let a = alias.trim();
    if a.is_empty() {
        ("skip", "could not determine the route".into())
    } else if a.to_lowercase().contains("tailscale") {
        ("ok", format!("via {a}"))
    } else {
        ("warn", format!("via {a}, not the Tailscale adapter: {VPN_HINT}"))
    }
}

/// Plain-language advice for a failed step.
pub fn hint_for(step: &str, detail: &str) -> String {
    let d = detail.to_lowercase();
    match step {
        "tailscale" => "Tailscale cannot reach that device. Check that Tailscale is running and signed in on both devices, and that the device is awake.".into(),
        "route" => VPN_HINT.into(),
        "tcp" => {
            if d.contains("refused") {
                "Port 22 is closed on the other device: turn on its SSH server (Mac: System Settings > General > Sharing > Remote Login; Windows: start the sshd service).".into()
            } else {
                "TCP 22 timed out although the Tailscale ping may work. If Shields Up is already off, suspect a Tailscale ACL / grant that does not allow tcp:22 between these two devices (admin console > Access controls), or a relay-only path (ping says \"via DERP\") that is dropping the connection; then check the other device's firewall and Remote Login / sshd. Also run the reverse test on the peer (see the Reverse line).".into()
            }
        }
        "ssh" => match classify_ssh_failure(detail) {
            SshFail::Auth => "The server refused this PC's key. Pair the devices again (Pair device), and on Windows run the elevated commands once.".into(),
            SshFail::HostKey => "The host key changed or was not recorded. Pair the devices again so the key is re-recorded.".into(),
            SshFail::Network => "SSH could not connect even though the port answered. Check the target address and that sshd is running.".into(),
            SshFail::Other => "SSH failed. See the detail line; try `ssh -vvv <target> true` in a terminal.".into(),
        },
        _ => "SSH works but Herdr did not answer. Make sure Herdr is running on the other device (herdr server) and the session name is right.".into(),
    }
}

#[derive(Debug, PartialEq)]
pub enum SshFail {
    Auth,
    HostKey,
    Network,
    Other,
}

pub fn classify_ssh_failure(stderr: &str) -> SshFail {
    let e = stderr.to_lowercase();
    if e.contains("permission denied") || e.contains("access is denied") {
        SshFail::Auth
    } else if e.contains("host key verification failed") || e.contains("remote host identification has changed") {
        SshFail::HostKey
    } else if e.contains("timed out") || e.contains("refused") || e.contains("no route") || e.contains("could not resolve") {
        SshFail::Network
    } else {
        SshFail::Other
    }
}

/// Parses `ssh -G` output into (hostname, port).
pub fn parse_ssh_g(out: &str) -> Option<(String, u16)> {
    let mut host = None;
    let mut port = None;
    for line in out.lines() {
        if let Some(v) = line.strip_prefix("hostname ") {
            host = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("port ") {
            port = v.trim().parse::<u16>().ok();
        }
    }
    Some((host.filter(|h| !h.is_empty())?, port.unwrap_or(22)))
}

/// Fallback when `ssh -G` is unavailable: `user@host:port` -> (host, port).
pub fn host_port_from_target(target: &str) -> (String, u16) {
    let (h, p) = split_target(target);
    let h = h.rsplit_once('@').map(|(_, h)| h).unwrap_or(h);
    (h.to_string(), p.unwrap_or(22))
}

/// One line per step: "ok  Tailscale ping - detail".
pub fn format_report(r: &DiagReport) -> String {
    let mut out = String::new();
    for st in &r.steps {
        let tag = match st.status.as_str() {
            "ok" => "ok  ",
            "warn" => "WARN",
            "skip" => "skip",
            _ => "FAIL",
        };
        out.push_str(&format!("{tag} {}", st.label));
        if !st.detail.is_empty() {
            out.push_str(&format!(" - {}", st.detail));
        }
        out.push('\n');
    }
    if let Some(h) = &r.hint {
        out.push_str(&format!("Hint: {h}\n"));
    }
    out
}

fn finish(steps: Vec<DiagStep>) -> DiagReport {
    let failed = steps.iter().find(|x| x.status == "fail");
    let hint = match failed {
        Some(f) => Some(hint_for(&f.step, &f.detail)),
        None => steps.iter().find(|x| x.status == "warn").map(|w| hint_for(&w.step, &w.detail)),
    };
    DiagReport { ok: failed.is_none(), steps, hint }
}

fn proc(cmd: &str) -> Proc {
    #[allow(unused_mut)]
    let mut c = Proc::new(cmd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Runs a command with a deadline; returns (success, stdout+stderr text).
fn run_timeout(mut c: Proc, secs: u64) -> Result<(bool, String), String> {
    let mut child = c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("timed out".into());
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let o = child.wait_with_output().map_err(|e| e.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    Ok((o.status.success(), text))
}

fn tailscale_cmd() -> Option<String> {
    if proc("tailscale").arg("version").output().is_ok() {
        return Some("tailscale".into());
    }
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into());
    let p = std::path::Path::new(&pf).join("Tailscale").join("tailscale.exe");
    p.exists().then(|| p.to_string_lossy().into_owned())
}

fn step_tailscale(host: &str) -> DiagStep {
    let Some(ts) = tailscale_cmd() else { return mk("tailscale", "skip", "tailscale command not found") };
    let mut c = proc(&ts);
    c.args(["ping", "-c", "1", "--timeout", "5s", host]);
    match run_timeout(c, 10) {
        Ok((true, t)) => mk("tailscale", "ok", t.lines().next().unwrap_or("pong").trim()),
        Ok((false, t)) => mk("tailscale", "fail", t.lines().next().unwrap_or("no reply").trim()),
        Err(e) => mk("tailscale", "fail", e),
    }
}

/// Windows only: the interface Windows picks for the peer's IP (Find-NetRoute). Read-only.
fn step_route(host: &str) -> DiagStep {
    if !cfg!(windows) {
        return mk("route", "skip", "only checked on Windows");
    }
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_hexdigit() || c == '.' || c == ':') {
        return mk("route", "skip", "peer is not an IP address");
    }
    let mut c = proc("powershell");
    c.args(["-NoProfile", "-NonInteractive", "-Command", &format!("(Find-NetRoute -RemoteIPAddress '{host}' | Select-Object -First 1).InterfaceAlias")]);
    match run_timeout(c, 10) {
        Ok((true, t)) => {
            let alias = t.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
            let (st, d) = route_verdict(alias);
            mk("route", st, d)
        }
        Ok((false, t)) => mk("route", "skip", t.lines().next().unwrap_or("Find-NetRoute failed").trim()),
        Err(e) => mk("route", "skip", e),
    }
}

fn own_tailscale_ip() -> Option<String> {
    let ts = tailscale_cmd()?;
    let mut c = proc(&ts);
    c.args(["ip", "-4"]);
    run_timeout(c, 5).ok().filter(|(ok, _)| *ok).and_then(|(_, t)| t.lines().next().map(|l| l.trim().to_string())).filter(|l| !l.is_empty())
}

/// The reverse direction cannot be probed from here (it needs a working ssh), so say exactly what to run on the peer.
fn step_reverse(port_hint: u16) -> DiagStep {
    let ip = own_tailscale_ip().unwrap_or_else(|| "<this device's Tailscale IP>".into());
    let _ = port_hint;
    mk("reverse", "skip", format!("not testable from here. On the peer run: Mac `nc -vz {ip} 22`, Windows `Test-NetConnection {ip} -Port 22`. If that also times out, it is a Tailscale ACL / relay-only path, not one device's firewall."))
}

fn step_tcp(host: &str, port: u16) -> DiagStep {
    let addrs = match (host, port).to_socket_addrs() {
        Ok(a) => a.collect::<Vec<_>>(),
        Err(e) => return mk("tcp", "fail", format!("cannot resolve {host}: {e}")),
    };
    let mut last = String::from("no address");
    for a in addrs {
        match TcpStream::connect_timeout(&a, Duration::from_secs(5)) {
            Ok(_) => return mk("tcp", "ok", format!("connected to {a}")),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock => last = format!("{a}: timed out after 5 s"),
            Err(e) => last = format!("{a}: {e}"),
        }
    }
    mk("tcp", "fail", last)
}

fn step_ssh(target: &str) -> DiagStep {
    let (host, port) = split_target(target);
    let alias = wooloo_core::meta::valid_alias(host);
    let mut c = proc("ssh");
    c.args(wooloo_core::meta::peer_conf_args(host));
    c.args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5"]);
    if let Some(p) = port {
        c.args(["-p", &p.to_string()]);
    }
    c.arg(host).arg("true");
    match run_timeout(c, 12) {
        Ok((true, _)) => mk("ssh", "ok", "logged in"),
        Ok((false, t)) => {
            let t = t.trim().to_string();
            // A paired peer's forced command rejects `true`, but only after a successful login.
            if alias && classify_ssh_failure(&t) == SshFail::Other {
                mk("ssh", "ok", "key accepted (peer dispatcher answered)")
            } else {
                mk("ssh", "fail", t.lines().last().unwrap_or("ssh failed").to_string())
            }
        }
        Err(e) => mk("ssh", "fail", e),
    }
}

fn step_bridge(target: &str, session: Option<String>) -> DiagStep {
    let args = match SshTransport::new(target.to_string(), session).args(Channel::Api) {
        Ok(a) => a,
        Err(e) => return mk("bridge", "fail", e.to_string()),
    };
    let mut child = match proc("ssh").args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => return mk("bridge", "fail", e.to_string()),
    };
    if let Some(mut i) = child.stdin.take() {
        let _ = i.write_all(b"{\"id\":\"diag\",\"method\":\"session.snapshot\",\"params\":{}}\n");
        let _ = i.flush();
        std::mem::forget(i); // keep the pipe open until we kill the child
    }
    let out = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(out).read_line(&mut line);
        let _ = tx.send(line);
    });
    let got = rx.recv_timeout(Duration::from_secs(10));
    let _ = child.kill();
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        use std::io::Read;
        let _ = e.read_to_string(&mut err);
    }
    let _ = child.wait();
    match got {
        Ok(line) if line.contains("\"diag\"") => mk("bridge", "ok", "Herdr answered"),
        Ok(line) if line.trim().is_empty() => mk("bridge", "fail", err.trim().lines().last().unwrap_or("connection closed with no answer").to_string()),
        Ok(line) => mk("bridge", "fail", format!("unexpected reply: {}", line.trim().chars().take(120).collect::<String>())),
        Err(_) => mk("bridge", "fail", "no answer within 10 s"),
    }
}

fn resolve(target: &str) -> (String, u16) {
    let (h, p) = split_target(target);
    let mut c = proc("ssh");
    c.args(wooloo_core::meta::peer_conf_args(h));
    c.arg("-G");
    if let Some(p) = p {
        c.args(["-p", &p.to_string()]);
    }
    c.arg(h);
    run_timeout(c, 5).ok().filter(|(ok, _)| *ok).and_then(|(_, t)| parse_ssh_g(&t)).unwrap_or_else(|| host_port_from_target(target))
}

fn run_all(target: String, session: Option<String>) -> DiagReport {
    let (host, port) = resolve(&target);
    let mut steps = Vec::new();
    let checks: [Box<dyn Fn() -> DiagStep>; 5] = [
        Box::new({ let h = host.clone(); move || step_tailscale(&h) }),
        Box::new({ let h = host.clone(); move || step_route(&h) }),
        Box::new({ let h = host.clone(); move || step_tcp(&h, port) }),
        Box::new({ let t = target.clone(); move || step_ssh(&t) }),
        Box::new({ let t = target.clone(); move || step_bridge(&t, session.clone()) }),
    ];
    for f in checks.iter() {
        let st = f();
        let failed = st.status == "fail";
        steps.push(st);
        if failed {
            break;
        }
    }
    // Reverse direction is shown after a TCP/SSH failure (the case where it matters).
    if steps.iter().any(|x| x.status == "fail" && (x.step == "tcp" || x.step == "ssh")) {
        steps.push(step_reverse(port));
    }
    finish(steps)
}

/// Read-only connectivity check of one SSH machine (sends no secrets, changes nothing).
#[tauri::command]
pub async fn test_connection(app: State<'_, App>, id: String) -> Res<DiagReport> {
    let m = {
        let f = app.file.lock().map_err(s)?;
        f.get(&id).cloned().ok_or_else(|| "unknown machine".to_string())?
    };
    if m.kind != MachineKind::Ssh {
        return Err("Test connection is for SSH machines only".into());
    }
    let target = m.ssh_target.clone().ok_or_else(|| "machine has no ssh target".to_string())?;
    tokio::task::spawn_blocking(move || run_all(target, m.session)).await.map_err(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(steps: Vec<DiagStep>) -> DiagReport {
        finish(steps)
    }

    #[test]
    fn tcp_timeout_hint_mentions_shields_up() {
        let r = rep(vec![mk("tailscale", "ok", "pong"), mk("tcp", "fail", "1.2.3.4:22: timed out after 5 s")]);
        assert!(!r.ok);
        let h = r.hint.unwrap();
        assert!(h.contains("Shields Up") && h.contains("ACL") && h.contains("relay"));
    }

    #[test]
    fn reverse_step_is_skip_and_names_command() {
        let s = step_reverse(22);
        assert_eq!(s.status, "skip");
        assert!(s.detail.contains("nc -vz") && s.detail.contains("Test-NetConnection"));
    }

    #[test]
    fn route_verdict_warns_off_tailscale_adapter() {
        assert_eq!(route_verdict("Tailscale").0, "ok");
        assert_eq!(route_verdict("tailscale0").0, "ok");
        let (st, d) = route_verdict("Wi-Fi VPN TUN");
        assert_eq!(st, "warn");
        assert!(d.contains("not the Tailscale adapter") && d.contains("100.64.0.0/10"));
        assert_eq!(route_verdict("  ").0, "skip");
    }

    #[test]
    fn warn_gives_hint_but_stays_ok() {
        let r = rep(vec![mk("tailscale", "ok", ""), mk("route", "warn", "via Foo")]);
        assert!(r.ok);
        assert_eq!(r.hint.as_deref(), Some(VPN_HINT));
        assert!(format_report(&r).contains("WARN Route to peer"));
    }

    #[test]
    fn tcp_refused_hint_mentions_sshd() {
        assert!(hint_for("tcp", "connection refused").contains("SSH server"));
    }

    #[test]
    fn all_ok_has_no_hint() {
        let r = rep(vec![mk("tailscale", "ok", ""), mk("tcp", "ok", ""), mk("ssh", "ok", ""), mk("bridge", "ok", "")]);
        assert!(r.ok && r.hint.is_none());
    }

    #[test]
    fn skip_is_not_failure() {
        assert!(rep(vec![mk("tailscale", "skip", "not found")]).ok);
    }

    #[test]
    fn format_lines() {
        let r = rep(vec![mk("tailscale", "ok", "pong"), mk("tcp", "fail", "timed out")]);
        let t = format_report(&r);
        assert!(t.starts_with("ok   Tailscale ping - pong\nFAIL TCP port 22 - timed out\nHint: "));
    }

    #[test]
    fn classify() {
        assert_eq!(classify_ssh_failure("risha@x: Permission denied (publickey)."), SshFail::Auth);
        assert_eq!(classify_ssh_failure("Host key verification failed."), SshFail::HostKey);
        assert_eq!(classify_ssh_failure("Connection timed out"), SshFail::Network);
        assert_eq!(classify_ssh_failure("wooloo-peer: request not allowed"), SshFail::Other);
    }

    #[test]
    fn parse_g() {
        assert_eq!(parse_ssh_g("user a\nhostname 100.1.2.3\nport 2222\n"), Some(("100.1.2.3".into(), 2222)));
        assert_eq!(parse_ssh_g("hostname h\n"), Some(("h".into(), 22)));
        assert_eq!(parse_ssh_g("user a\n"), None);
    }

    #[test]
    fn target_fallback() {
        assert_eq!(host_port_from_target("zomato@100.121.45.79"), ("100.121.45.79".into(), 22));
        assert_eq!(host_port_from_target("a@h:2200"), ("h".into(), 2200));
    }
}
