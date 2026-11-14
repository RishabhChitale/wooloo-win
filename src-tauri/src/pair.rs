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
    h.path().app_config_dir().map_err(s)
}

fn home() -> Res<PathBuf> {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).ok_or_else(|| "no home directory".to_string())
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Proc::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn herdr_version() -> Res<String> {
    let t = run("herdr", &["--version"]).ok_or("herdr not found on PATH (needed to read its version)")?;
    let v = t.lines().next().unwrap_or("").trim().trim_start_matches("herdr").trim().to_string();
    if v.is_empty() { Err("herdr --version printed nothing".into()) } else { Ok(v) }
}

fn herdr_path() -> Option<String> {
    run("where.exe", &["herdr.exe"]).and_then(|t| t.lines().next().map(|l| l.trim().to_string())).filter(|l| !l.is_empty())
}

fn host_key() -> Res<String> {
    let pd = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".into());
    let p = Path::new(&pd).join("ssh").join("ssh_host_ed25519_key.pub");
    let t = std::fs::read_to_string(&p).map_err(|e| format!("cannot read {} (is OpenSSH Server installed and started?): {e}", p.display()))?;
    pairing::key_of(&t).map_err(s)
}

fn is_admin() -> bool {
    run("whoami", &["/groups"]).map(|t| t.contains("S-1-5-32-544")).unwrap_or(false)
}

#[derive(Serialize)]
pub struct LocalInfo {
    code: String,
    peer_id: String,
    public_fp: String,
    host_fp: String,
}

/// Builds this device's pairing code to paste into the other device. `target` is `user@host[:port]` as the peer
/// should reach this PC (its Tailscale name or 100.x address).
#[tauri::command(rename_all = "snake_case")]
pub fn pair_local_info(app: AppHandle, target: String, session: Option<String>, label: Option<String>) -> Res<LocalInfo> {
    let key_path = config_dir(&app)?.join(KEY_FILE);
    let public_key = pairing::ensure_device_key(&key_path).map_err(s)?;
    let host_key = host_key()?;
    let label = label.filter(|l| !l.trim().is_empty()).or_else(|| std::env::var("COMPUTERNAME").ok()).unwrap_or_else(|| "Windows PC".into());
    let c = PairCode {
        target: target.trim().to_string(),
        public_key: public_key.clone(),
        host_key: host_key.clone(),
        herdr_version: herdr_version()?,
        session: session.filter(|x| !x.trim().is_empty()).unwrap_or_else(|| "wooloo".into()),
        label: label.trim().to_string(),
    };
    Ok(LocalInfo {
        code: c.encode().map_err(s)?,
        peer_id: c.peer_id(),
        public_fp: pairing::fingerprint(&public_key).map_err(s)?,
        host_fp: pairing::fingerprint(&host_key).map_err(s)?,
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
    })
}

#[derive(Serialize)]
pub struct Accepted {
    machine: Machine,
    peer_id: String,
    /// Set when the authorized_keys file could not be written (not elevated): add this line by hand, see docs.
    needs_elevation: Option<String>,
    authorized_keys_path: String,
}

fn write_keys(path: &Path, peer: &str, line: &str, admin: bool) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let (text, changed) = pairing::merge_authorized_keys(&old, peer, line);
    if changed {
        std::fs::write(path, text)?;
    }
    if admin {
        // sshd ignores administrators_authorized_keys unless only SYSTEM and Administrators may touch it.
        let ok = Proc::new("icacls")
            .arg(path)
            .args(["/inheritance:r", "/grant", "SYSTEM:F", "/grant", "Administrators:F"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "icacls failed"));
        }
    }
    Ok(())
}

/// Installs the pairing on this PC after the user confirmed the fingerprints in the dialog.
#[tauri::command(rename_all = "snake_case")]
pub fn pair_accept(app: AppHandle, state: State<App>, code: String, include_ssh_config: bool) -> Res<Accepted> {
    let c = PairCode::decode(&code).map_err(s)?;
    let peer = c.peer_id();
    let cfg = config_dir(&app)?;
    std::fs::create_dir_all(&cfg).map_err(s)?;

    // 1. dispatcher: copy the bundled script into the app config dir
    let src = app.path().resolve(format!("resources/{DISPATCHER}"), BaseDirectory::Resource).map_err(s)?;
    let dst = cfg.join(DISPATCHER);
    std::fs::copy(&src, &dst).map_err(|e| format!("cannot install dispatcher from {}: {e}", src.display()))?;

    // 2. authorized_keys entry for the peer's key (forced command, restrict)
    let admin = is_admin();
    let ak = authorized_keys_path(admin)?;
    let hp = herdr_path();
    let line = pairing::authorized_keys_line(&dst.to_string_lossy(), &peer, hp.as_deref(), &c.public_key).map_err(s)?;
    let needs_elevation = write_keys(&ak, &peer, &line, admin).err().map(|_| line.clone());

    // 3. ssh files for this PC -> peer
    let dir = home()?.join(".ssh").join("wooloo_peers");
    std::fs::create_dir_all(&dir).map_err(s)?;
    let kh = dir.join(format!("{peer}.known_hosts"));
    std::fs::write(&kh, pairing::known_hosts_line(&peer, &c.host_key)).map_err(s)?;
    let conf = pairing::ssh_conf(&peer, &c.target, &cfg.join(KEY_FILE).to_string_lossy(), &kh.to_string_lossy()).map_err(s)?;
    std::fs::write(dir.join(format!("{peer}.conf")), conf).map_err(s)?;
    if include_ssh_config {
        let cp = home()?.join(".ssh").join("config");
        if std::fs::symlink_metadata(&cp).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            return Err("~/.ssh/config is a symlink; not touching it (Include line skipped, everything else done)".into());
        }
        let old = std::fs::read_to_string(&cp).unwrap_or_default();
        let new = pairing::with_include(&old);
        if new != old {
            std::fs::write(&cp, new).map_err(s)?;
        }
    }

    // 4. the machine in machines.toml (re-pairing replaces the old entry), started right away
    let host_alias = format!("wooloo-peer-{peer}");
    let m = {
        let mut f = state.file.lock().unwrap();
        let old: Vec<String> = f.machines.iter().filter(|m| m.ssh_target.as_deref() == Some(host_alias.as_str())).map(|m| m.id.clone()).collect();
        for id in old {
            f.remove(&id);
        }
        let m = f.add_ssh(c.label.clone(), host_alias, Some(c.session.clone())).map_err(s)?;
        f.save(&state.path).map_err(s)?;
        m
    };
    state.start(&m);
    Ok(Accepted { machine: m, peer_id: peer, needs_elevation, authorized_keys_path: ak.display().to_string() })
}
