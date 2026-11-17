//! "Pair device" commands: the Windows half of the Mac<->Windows peer pairing.
//! Logic that can be tested without a machine lives in `wooloo_core::pairing`; this file does the I/O.
use super::{s, App, Res};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager, State};
use wooloo_core::machines::Machine;
use wooloo_core::pairing::{self, PairCode};

const KEY_FILE: &str = "wooloo_peer_ed25519";
const DISPATCHER: &str = "peer-dispatch.ps1";

fn config_dir(h: &AppHandle) -> Res<PathBuf> {
    h.path().app_config_dir().map_err(|e| format!("cannot find the wooloo config directory: {e}"))
}

/// Re-copy the bundled dispatcher over an installed one, so an app update takes effect without re-pairing.
/// Only when a dispatcher is already installed (i.e. this PC has been paired); failures are ignored.
pub fn refresh_dispatcher(app: &AppHandle) {
    let Ok(cfg) = config_dir(app) else { return };
    let dst = cfg.join(DISPATCHER);
    if !dst.exists() {
        return;
    }
    copy_bundled(app, &cfg);
}

const SYNC_BIN: &str = "wooloo-sync.exe";

/// Copy the bundled dispatcher and sync responder into the config dir (errors ignored: best effort).
fn copy_bundled(app: &AppHandle, cfg: &std::path::Path) {
    for name in [DISPATCHER, SYNC_BIN] {
        if let Ok(src) = app.path().resolve(format!("resources/{name}"), BaseDirectory::Resource) {
            // the repo carries an empty placeholder; the release build replaces it with the real binary
            if std::fs::metadata(&src).map(|m| m.len() > 0).unwrap_or(false) {
                let _ = std::fs::copy(&src, cfg.join(name));
            }
        }
    }
}

fn home() -> Res<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).ok_or_else(|| "no home directory".to_string())
}

/// A child process with no console window (this is a GUI app).
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

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let o = proc(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Error text for a file operation: what we tried, the path and the OS error.
fn io<'a>(what: &'a str, path: &'a Path) -> impl Fn(std::io::Error) -> String + 'a {
    move |e| format!("cannot {what} {}: {e}", path.display())
}

fn herdr_version() -> Res<String> {
    let mut r = wooloo_core::resolve::resolve_herdr();
    if r.is_err() {
        wooloo_core::resolve::invalidate();
        r = wooloo_core::resolve::resolve_herdr();
    }
    let r = r?;
    let exe = r.path.to_string_lossy().into_owned();
    let t = run(&exe, &["--version"]).ok_or_else(|| format!("cannot run `{exe} --version`: it failed. Resolved via:\n  {}", r.tried.join("\n  ")))?;
    let v = t.lines().next().unwrap_or("").trim().trim_start_matches("herdr").trim().to_string();
    if v.is_empty() { Err("`herdr --version` printed nothing".into()) } else { Ok(v) }
}

fn herdr_path() -> Option<String> {
    wooloo_core::resolve::resolve_herdr().ok().map(|r| r.path.to_string_lossy().into_owned())
}

/// `ssh-keyscan -t ed25519 <addr>`; Err text = exit status + first stderr line (or the spawn error).
fn keyscan(addr: &str) -> Result<String, String> {
    let o = proc("ssh-keyscan")
        .args(["-T", "5", "-t", "ed25519", addr])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot run ssh-keyscan (OpenSSH client missing?): {e}"))?;
    let out = String::from_utf8_lossy(&o.stdout);
    if o.status.success() {
        if let Some(k) = pairing::parse_keyscan(&out) {
            return Ok(k);
        }
    }
    let stderr = String::from_utf8_lossy(&o.stderr);
    let first = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let why = if o.status.success() { "no ed25519 key in output" } else { "" };
    Err(format!("{} {why} {first}", o.status).split_whitespace().collect::<Vec<_>>().join(" "))
}

/// This PC's sshd host key: ssh-keyscan on loopback first (any user may run it); the key file only as a fallback,
/// because C:\ProgramData\ssh\ssh_host_ed25519_key.pub is admin-only on Windows.
fn host_key() -> Res<String> {
    let pd = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".into());
    let p = Path::new(&pd).join("ssh").join("ssh_host_ed25519_key.pub");
    pairing::resolve_host_key(
        keyscan,
        || {
            let attempt = |e: String| pairing::Attempt { what: String::new(), err: e };
            let t = std::fs::read_to_string(&p).map_err(|e| attempt(e.to_string()))?;
            pairing::key_of(&t).map_err(|e| attempt(e.to_string()))
        },
        &p.display().to_string(),
    )
}

/// (member of the Administrators group, running elevated). sshd reads administrators_authorized_keys for members.
fn admin_flags() -> (bool, bool) {
    run("whoami", &["/groups"]).map(|t| pairing::admin_flags(&t)).unwrap_or((false, false))
}

/// Writing C:\\ProgramData\\ssh\\administrators_authorized_keys and icacls need elevation; ~/.ssh/authorized_keys does not.
fn is_admin() -> bool {
    admin_flags().0
}

#[derive(Serialize)]
pub struct LocalInfo {
    code: String,
    peer_id: String,
    public_fp: String,
    host_fp: String,
    /// Absolute path of this PC's herdr executable (resolved from the registry / installer dirs), for display.
    herdr_path: Option<String>,
}

#[derive(Serialize)]
pub struct Defaults {
    /// `user@host` to prefill; the user may edit it. Empty parts are left out.
    target: String,
    user: String,
    host: String,
    /// What was found or what is missing / wrong, for the dialog to show.
    notes: Vec<String>,
}

/// Runs `tailscale` from PATH, then from the default Windows install folder.
fn tailscale(args: &[&str]) -> Option<String> {
    run("tailscale", args).or_else(|| {
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into());
        run(&Path::new(&pf).join("Tailscale").join("tailscale.exe").to_string_lossy(), args)
    })
}

/// Prefill for the Pair dialog's target box: this PC's Tailscale IPv4 (else its MagicDNS name) and the login from `whoami`.
#[tauri::command]
pub fn pair_defaults() -> Defaults {
    let mut notes = Vec::new();
    let host = match tailscale(&["ip", "-4"]).and_then(|t| pairing::parse_tailscale_ip(&t)) {
        Some(ip) => ip,
        None => match tailscale(&["status", "--json"]).and_then(|t| pairing::parse_magicdns(&t)) {
            Some(n) => {
                notes.push(format!("No Tailscale IPv4 from `tailscale ip -4`; using the MagicDNS name {n}."));
                n
            }
            None => {
                notes.push("Could not read this PC's Tailscale address (`tailscale ip -4` and `tailscale status --json` failed). Is Tailscale installed and signed in? Type the host as user@100.x.y.z.".into());
                String::new()
            }
        },
    };
    let user = match run("whoami", &[]) {
        Some(t) => {
            let u = pairing::login_from_whoami(&t);
            match pairing::check_login(&u) {
                Ok(()) => u,
                Err(e) => {
                    notes.push(e.to_string());
                    String::new()
                }
            }
        }
        None => {
            notes.push("Could not run `whoami` to find the Windows login; type it in the target box.".into());
            String::new()
        }
    };
    let target = match (user.is_empty(), host.is_empty()) {
        (false, false) => format!("{user}@{host}"),
        (false, true) => format!("{user}@"),
        (true, false) => format!("@{host}"),
        (true, true) => String::new(),
    };
    Defaults { target, user, host, notes }
}

/// Builds this device's pairing code to paste into the other device. `target` is `user@host[:port]` as the peer
/// should reach this PC (its Tailscale name or 100.x address).
#[tauri::command(rename_all = "snake_case")]
pub fn pair_local_info(app: AppHandle, target: String, session: Option<String>, label: Option<String>) -> Res<LocalInfo> {
    let target = target.trim().to_string();
    pairing::check_target(&target, "the Target box (this PC as the peer reaches it)", false).map_err(s)?;
    let key_path = config_dir(&app)?.join(KEY_FILE);
    let public_key = pairing::ensure_device_key(&key_path).map_err(s)?; // errors name the path / ssh-keygen command
    let host_key = host_key()?;
    let label = label.filter(|l| !l.trim().is_empty()).or_else(|| std::env::var("COMPUTERNAME").ok()).unwrap_or_else(|| "Windows PC".into());
    let c = PairCode {
        target,
        public_key: public_key.clone(),
        host_key: host_key.clone(),
        herdr_version: herdr_version()?,
        session: match session.filter(|x| !x.trim().is_empty()) {
            Some(x) => x,
            None => tauri::async_runtime::block_on(wooloo_core::discover::detect_pairing_session()), // the session the running Herdr really uses
        },
        label: label.trim().to_string(),
    };
    Ok(LocalInfo {
        code: c.encode().map_err(s)?,
        peer_id: c.peer_id(),
        public_fp: pairing::fingerprint(&public_key).map_err(s)?,
        host_fp: pairing::fingerprint(&host_key).map_err(s)?,
        herdr_path: herdr_path(),
    })
}

#[derive(Serialize)]
pub struct Review {
    label: String,
    target: String,
    session: String,
    herdr_version: String,
    peer_id: String,
    public_fp: String,
    host_fp: String,
    admin: bool,
    authorized_keys_path: String,
    /// Absolute path of this PC's herdr executable, for display (None if not found).
    herdr_path: Option<String>,
}

fn authorized_keys_path(admin: bool) -> Res<PathBuf> {
    if admin {
        let pd = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".into());
        Ok(Path::new(&pd).join("ssh").join("administrators_authorized_keys"))
    } else {
        Ok(home()?.join(".ssh").join("authorized_keys"))
    }
}

/// Decodes and validates a pasted code and returns what the user must check before accepting.
#[tauri::command]
pub fn pair_review(code: String) -> Res<Review> {
    let c = PairCode::decode(&code).map_err(s)?;
    let admin = is_admin();
    Ok(Review {
        peer_id: c.peer_id(),
        public_fp: pairing::fingerprint(&c.public_key).map_err(s)?,
        host_fp: pairing::fingerprint(&c.host_key).map_err(s)?,
        authorized_keys_path: authorized_keys_path(admin)?.display().to_string(),
        label: c.label,
        target: c.target,
        session: c.session,
        herdr_version: c.herdr_version,
        admin,
        herdr_path: herdr_path(),
    })
}

#[derive(Serialize)]
pub struct Accepted {
    machine: Machine,
    peer_id: String,
    /// Set when the authorized_keys file could not be written (not elevated): add this line by hand, see docs.
    needs_elevation: Option<String>,
    /// PowerShell to paste into an elevated prompt (same condition as `needs_elevation`).
    elevated_commands: Option<String>,
    /// Why the write was skipped or failed (not elevated / the OS error), when `needs_elevation` is set.
    elevation_reason: Option<String>,
    authorized_keys_path: String,
    /// Ids of earlier rows for this same peer that were replaced (the UI drops them).
    replaced: Vec<String>,
}

enum KeysOutcome {
    Written,
    /// Needs an elevated prompt; the string says why (path + OS error).
    NeedsElevation(String),
}

fn write_keys(path: &Path, peer: &str, line: &str, admin: bool, elevated: bool) -> Res<KeysOutcome> {
    use std::io::ErrorKind::PermissionDenied;
    if admin && !elevated {
        return Ok(KeysOutcome::NeedsElevation(format!(
            "not running elevated: writing {} and setting its ACL (icacls) need administrator rights",
            path.display()
        )));
    }
    let denied = |what: &str, e: std::io::Error| -> Res<KeysOutcome> {
        let msg = format!("cannot {what} {}: {e}", path.display());
        if e.kind() == PermissionDenied { Ok(KeysOutcome::NeedsElevation(msg)) } else { Err(msg) }
    };
    if let Some(d) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(d) {
            return denied("create directory for", e);
        }
    }
    let old = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return denied("read", e),
    };
    let (text, changed) = pairing::merge_authorized_keys(&old, peer, line);
    if changed {
        if let Err(e) = std::fs::write(path, text) {
            return denied("write", e);
        }
    }
    if admin {
        // sshd ignores administrators_authorized_keys unless only SYSTEM and Administrators may touch it.
        let o = proc("icacls")
            .arg(path)
            .args(["/inheritance:r", "/grant", "SYSTEM:F", "/grant", "Administrators:F"])
            .output()
            .map_err(|e| format!("cannot run `icacls {}`: {e}", path.display()))?;
        if !o.status.success() {
            let msg = format!(
                "`icacls {} /inheritance:r /grant SYSTEM:F /grant Administrators:F` failed ({}): {}{}",
                path.display(),
                o.status,
                String::from_utf8_lossy(&o.stderr).trim(),
                String::from_utf8_lossy(&o.stdout).trim()
            );
            return Ok(KeysOutcome::NeedsElevation(msg));
        }
    }
    Ok(KeysOutcome::Written)
}

/// Installs the pairing on this PC after the user confirmed the fingerprints in the dialog.
#[tauri::command(rename_all = "snake_case")]
pub fn pair_accept(app: AppHandle, state: State<App>, code: String, include_ssh_config: bool) -> Res<Accepted> {
    // Never let a panic take the app down: turn it into an error shown in the dialog.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pair_accept_inner(&app, &state, code, include_ssh_config)))
        .unwrap_or_else(|p| Err(format!("pairing failed unexpectedly: {} (details in crash.log under %APPDATA%\\wooloo)", panic_text(p.as_ref()))))
}

fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown panic".into())
}

fn pair_accept_inner(app: &AppHandle, state: &State<App>, code: String, include_ssh_config: bool) -> Res<Accepted> {
    let app = app.clone();
    let c = PairCode::decode(&code).map_err(s)?;
    let peer = c.peer_id();
    let cfg = config_dir(&app)?;
    std::fs::create_dir_all(&cfg).map_err(io("create directory", &cfg))?;

    // 1. dispatcher: copy the bundled script into the app config dir
    let src = app.path().resolve(format!("resources/{DISPATCHER}"), BaseDirectory::Resource).map_err(|e| format!("cannot locate bundled resources/{DISPATCHER}: {e}"))?;
    let dst = cfg.join(DISPATCHER);
    std::fs::copy(&src, &dst).map_err(|e| format!("cannot copy dispatcher {} to {}: {e}", src.display(), dst.display()))?;
    copy_bundled(&app, &cfg);

    // 2. authorized_keys entry for the peer's key (forced command, restrict)
    let (admin, elevated) = admin_flags();
    let ak = authorized_keys_path(admin)?;
    let hp = herdr_path();
    let line = pairing::authorized_keys_line(&dst.to_string_lossy(), &peer, hp.as_deref(), &c.public_key).map_err(s)?;
    // Not elevated: never fail the pairing. Hand the user the exact line and the elevated commands instead.
    let (needs_elevation, elevated_commands, elevation_reason) = match write_keys(&ak, &peer, &line, admin, elevated)? {
        KeysOutcome::Written => (None, None, None),
        KeysOutcome::NeedsElevation(why) => {
            (Some(line.clone()), Some(pairing::elevated_commands(&ak.display().to_string(), &line)), Some(why))
        }
    };

    // 3. ssh files for this PC -> peer
    let dir = home()?.join(".ssh").join("wooloo_peers");
    std::fs::create_dir_all(&dir).map_err(io("create directory", &dir))?;
    let kh = dir.join(format!("{peer}.known_hosts"));
    std::fs::write(&kh, pairing::known_hosts_line(&peer, &c.host_key)).map_err(io("write", &kh))?;
    let conf = pairing::ssh_conf(&peer, &c.target, &cfg.join(KEY_FILE).to_string_lossy(), &kh.to_string_lossy()).map_err(s)?;
    let confp = dir.join(format!("{peer}.conf"));
    std::fs::write(&confp, conf).map_err(io("write", &confp))?;
    if include_ssh_config {
        let cp = home()?.join(".ssh").join("config");
        if std::fs::symlink_metadata(&cp).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            return Err("~/.ssh/config is a symlink; not touching it (Include line skipped, everything else done)".into());
        }
        let old = std::fs::read_to_string(&cp).unwrap_or_default();
        let new = pairing::with_include(&old);
        if new != old {
            std::fs::write(&cp, new).map_err(io("write", &cp))?;
        }
    }

    // 4. the machine in machines.toml (re-pairing replaces the old entry), started right away
    let host_alias = format!("wooloo-peer-{peer}");
    let (m, replaced) = {
        let mut f = state.file.lock().unwrap_or_else(|e| e.into_inner());
        let old: Vec<String> = f.machines.iter().filter(|m| m.ssh_target.as_deref() == Some(host_alias.as_str())).map(|m| m.id.clone()).collect();
        for id in &old {
            f.remove(id);
        }
        let m = f.add_ssh(c.label.clone(), host_alias, Some(c.session.clone())).map_err(s)?;
        f.save(&state.path).map_err(|e| format!("cannot save {}: {e}", state.path.display()))?;
        (m, old)
    };
    {
        let mut h = state.handles.lock().unwrap_or_else(|e| e.into_inner());
        for id in &replaced {
            if let Some(h) = h.remove(id) {
                h.send(wooloo_core::manager::Command::Stop);
            }
        }
    }
    state.start(&m);
    Ok(Accepted { replaced, machine: m, peer_id: peer, needs_elevation, elevated_commands, elevation_reason, authorized_keys_path: ak.display().to_string() })
}
