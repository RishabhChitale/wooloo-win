//! Peer pairing (`wooloo-pair-v1`), the Windows half of the Mac<->Windows pairing in the Mac repo's
//! `docs/peer-pairing.md`. Pure code (no I/O) plus small helpers that drive the system `ssh-keygen`.
//!
//! A pairing code is `wooloo-pair-v1:` + base64url (no padding) of a JSON object with sorted keys:
//! `target`, `publicKey`, `hostKey`, `herdrVersion`, `session`, `label`. Decoding always validates.
use crate::{Error, Result};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const CODE_PREFIX: &str = "wooloo-pair-v1:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairCode {
    /// `user@host[:port]` or `user@[ipv6]:port`.
    pub target: String,
    /// `type blob` of the device's public key.
    pub public_key: String,
    /// `type blob` of the device's sshd host key.
    pub host_key: String,
    pub herdr_version: String,
    pub session: String,
    pub label: String,
}

fn bad<T>(m: &str) -> Result<T> {
    Err(Error::Other(format!("pairing code: {m}")))
}

/// `host`, `user` and `port` parts of a validated target.
pub fn split_target(t: &str) -> Result<(String, String, u16)> {
    let (user, rest) = match t.split_once('@') {
        Some((u, r)) if !u.is_empty() && !r.contains('@') => (u, r),
        _ => return bad("target must be user@host"),
    };
    let (host, port) = if let Some(r) = rest.strip_prefix('[') {
        let (h, tail) = r.split_once(']').ok_or_else(|| Error::Other("pairing code: bad ipv6 target".into()))?;
        match tail {
            "" => (h.to_string(), 22u16),
            _ => match tail.strip_prefix(':').and_then(|p| p.parse::<u16>().ok()) {
                Some(p) => (h.to_string(), p),
                None => return bad("bad port"),
            },
        }
    } else if let Some((h, p)) = rest.rsplit_once(':') {
        match p.parse::<u16>() {
            Ok(p) => (h.to_string(), p),
            Err(_) => return bad("bad port"),
        }
    } else {
        (rest.to_string(), 22)
    };
    if host.is_empty() || user.is_empty() || port == 0 {
        return bad("empty host, user or port");
    }
    Ok((user.to_string(), host, port))
}

/// Checks `type blob`: known type, base64 blob that starts with the same type string.
pub fn valid_key(k: &str) -> bool {
    let mut it = k.split(' ');
    let (Some(ty), Some(blob), None) = (it.next(), it.next(), it.next()) else { return false };
    if !matches!(ty, "ssh-ed25519" | "ssh-rsa" | "ecdsa-sha2-nistp256" | "ecdsa-sha2-nistp384" | "ecdsa-sha2-nistp521") {
        return false;
    }
    let Ok(raw) = STANDARD.decode(blob) else { return false };
    // wire format: uint32 length, type string, ...
    raw.len() > 4 + ty.len()
        && raw[..4] == (ty.len() as u32).to_be_bytes()
        && &raw[4..4 + ty.len()] == ty.as_bytes()
}

impl PairCode {
    pub fn validate(&self) -> Result<()> {
        let t = &self.target;
        if t.is_empty()
            || t.len() > 255
            || t.starts_with('-')
            || !t.bytes().all(|b| b.is_ascii_alphanumeric() || b"._@:[]-".contains(&b))
        {
            return bad("bad target");
        }
        split_target(t)?;
        if !valid_key(&self.public_key) {
            return bad("bad publicKey");
        }
        if !valid_key(&self.host_key) {
            return bad("bad hostKey");
        }
        let v = &self.herdr_version;
        if v.is_empty() || v.len() > 32 || !v.bytes().all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b)) {
            return bad("bad herdrVersion");
        }
        let s = &self.session;
        if s.is_empty() || s.len() > 64 || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return bad("bad session");
        }
        if self.label.is_empty() || self.label.chars().count() > 64 || self.label.chars().any(|c| c.is_control()) {
            return bad("bad label");
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<String> {
        self.validate()?;
        // serde_json::Value keeps keys sorted (no preserve_order feature).
        let v = serde_json::to_value(self).map_err(|e| Error::Other(e.to_string()))?;
        Ok(format!("{CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(v.to_string())))
    }

    pub fn decode(code: &str) -> Result<Self> {
        let body = match code.trim().strip_prefix(CODE_PREFIX) {
            Some(b) => b,
            None => return bad("not a wooloo-pair-v1 code"),
        };
        let raw = URL_SAFE_NO_PAD.decode(body).map_err(|_| Error::Other("pairing code: bad base64".into()))?;
        let c: PairCode = serde_json::from_slice(&raw).map_err(|e| Error::Other(format!("pairing code: {e}")))?;
        c.validate()?;
        Ok(c)
    }

    pub fn peer_id(&self) -> String {
        peer_id(&self.label)
    }
}

/// Label lowercased to `[a-z0-9-]` (other runs become one `-`), at most 32 chars, fallback `device`.
pub fn peer_id(label: &str) -> String {
    let mut out = String::new();
    for c in label.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(32).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "device".into() } else { out }
}

/// Quote for `command="..."` inside an authorized_keys line: only `"` is escaped (as `\"`).
fn cmd_quote(s: &str) -> String {
    s.replace('"', "\\\"")
}

/// Forward slashes avoid backslash escapes; PowerShell and OpenSSH on Windows accept them.
pub fn slash_path(p: &str) -> String {
    p.replace('\\', "/")
}

/// One authorized_keys line that forces the peer's key through the dispatcher.
/// `herdr` is the paired herdr.exe path (`None` = dispatcher searches PATH). Paths with spaces are quoted.
pub fn authorized_keys_line(dispatcher: &str, peer: &str, herdr: Option<&str>, public_key: &str) -> Result<String> {
    if !valid_key(public_key) {
        return bad("bad publicKey");
    }
    if peer.is_empty() || !peer.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return bad("bad peer id");
    }
    let q = |p: &str| -> Result<String> {
        let p = slash_path(p);
        if p.chars().any(|c| c.is_control()) {
            return bad("control characters in path");
        }
        Ok(if p.contains(' ') { format!("\"{p}\"") } else { p })
    };
    let mut cmd = format!("powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File {} -Peer {peer}", q(dispatcher)?);
    if let Some(h) = herdr {
        cmd.push_str(&format!(" -Herdr {}", q(h)?));
    }
    Ok(format!("command=\"{}\",restrict {public_key}", cmd_quote(&cmd)))
}

/// Adds `line` to an authorized_keys text unless the same key already has an entry. Returns the new text and
/// whether it changed. Replaces an earlier wooloo entry for the same peer id (re-pairing).
pub fn merge_authorized_keys(existing: &str, peer: &str, line: &str) -> (String, bool) {
    let key = line.rsplit(' ').take(2).collect::<Vec<_>>();
    let blob = key.first().copied().unwrap_or("");
    let marker = format!("peer-dispatch.ps1 -Peer {peer}");
    let mut out: Vec<&str> = Vec::new();
    for l in existing.lines() {
        if l == line {
            return (existing.to_string(), false);
        }
        if l.contains(&marker) && l.starts_with("command=") {
            continue;
        }
        if !blob.is_empty() && l.ends_with(blob) && !l.starts_with("command=") {
            // the same key already has an unrestricted entry: leave it exactly as it was
            return (existing.to_string(), false);
        }
        out.push(l);
    }
    out.push(line);
    (out.join("\r\n") + "\r\n", true)
}

/// `~/.ssh/wooloo_peers/<id>.known_hosts` content.
pub fn known_hosts_line(peer: &str, host_key: &str) -> String {
    format!("wooloo-peer-{peer} {host_key}\n")
}

/// `~/.ssh/wooloo_peers/<id>.conf` content. Paths use forward slashes.
pub fn ssh_conf(peer: &str, target: &str, identity: &str, known_hosts: &str) -> Result<String> {
    let (user, host, port) = split_target(target)?;
    Ok(format!(
        "Host wooloo-peer-{peer}\n    HostName {host}\n    User {user}\n    Port {port}\n    IdentityFile {}\n    IdentitiesOnly yes\n    HostKeyAlias wooloo-peer-{peer}\n    UserKnownHostsFile {}\n    StrictHostKeyChecking yes\n",
        slash_path(identity),
        slash_path(known_hosts)
    ))
}

pub const INCLUDE_LINE: &str = "Include ~/.ssh/wooloo_peers/*.conf";

/// Puts the Include line first in an ssh config text, once.
pub fn with_include(config: &str) -> String {
    if config.lines().any(|l| l.trim() == INCLUDE_LINE) {
        config.to_string()
    } else {
        format!("{INCLUDE_LINE}\n{config}")
    }
}

/// Addresses tried, in order, by `ssh-keyscan -t ed25519 <addr>` to read this PC's own sshd host key
/// (readable by any user, unlike C:\\ProgramData\\ssh\\ssh_host_ed25519_key.pub).
pub const KEYSCAN_HOSTS: [&str; 3] = ["127.0.0.1", "::1", "localhost"];

/// The ed25519 `type blob` from `ssh-keyscan -t ed25519` output. Lines look like `127.0.0.1 ssh-ed25519 AAAA...`
/// (host may be `[host]:port` or hashed); `# host:22 SSH-2.0-OpenSSH_9.5` comment lines, blank lines and other
/// key types (rsa, ecdsa) are skipped. None if no valid ed25519 key is present.
pub fn parse_keyscan(out: &str) -> Option<String> {
    out.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).find_map(|l| {
        let toks: Vec<&str> = l.split_whitespace().collect();
        let i = toks.iter().take(2).position(|t| *t == "ssh-ed25519")?;
        let blob = toks.get(i + 1)?;
        let k = format!("ssh-ed25519 {blob}");
        valid_key(&k).then_some(k)
    })
}

/// One attempt to learn the host key, for building the error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// What was tried, e.g. `ssh-keyscan -t ed25519 127.0.0.1` or the file path.
    pub what: String,
    /// Exit status / stderr first line / OS error.
    pub err: String,
}

/// Host key lookup order as a pure function: each keyscan address in order, then (only if all failed) the file.
/// `keyscan` and `file` are the actual I/O, called lazily; the first success wins and later ones are not run.
pub fn resolve_host_key(
    mut keyscan: impl FnMut(&str) -> std::result::Result<String, String>,
    file: impl FnOnce() -> std::result::Result<String, Attempt>,
    file_path: &str,
) -> std::result::Result<String, String> {
    let mut tried: Vec<Attempt> = Vec::new();
    for h in KEYSCAN_HOSTS {
        match keyscan(h) {
            Ok(k) => return Ok(k),
            Err(e) => tried.push(Attempt { what: format!("ssh-keyscan -t ed25519 {h}"), err: e }),
        }
    }
    match file() {
        Ok(k) => Ok(k),
        Err(a) => {
            tried.push(Attempt { what: format!("read {file_path}"), ..a });
            Err(host_key_error(&tried))
        }
    }
}

/// Error text naming every attempt and the exact fix.
pub fn host_key_error(tried: &[Attempt]) -> String {
    let list: Vec<String> = tried.iter().map(|a| format!("{}: {}", a.what, a.err)).collect();
    format!(
        "cannot get this PC's SSH host key. Tried: {}. Fix: start the OpenSSH server (PowerShell as Administrator: Start-Service sshd), \
         install the OpenSSH client (ssh-keyscan), or run wooloo as administrator once.",
        list.join(" | ")
    )
}

/// PowerShell for an elevated prompt that appends `line` to `path` (administrators_authorized_keys) and locks
/// its ACL the way sshd requires. Single quotes are doubled so any line is a safe PowerShell literal.
pub fn elevated_commands(path: &str, line: &str) -> String {
    let q = |s: &str| s.replace('\'', "''");
    format!(
        "$p = '{}'\nNew-Item -ItemType Directory -Force -Path (Split-Path $p) | Out-Null\nAdd-Content -Path $p -Value '{}' -Encoding ascii\nicacls $p /inheritance:r /grant 'SYSTEM:F' /grant 'Administrators:F'",
        q(path),
        q(line)
    )
}

/// Parsed `whoami /groups`: (member of the Administrators group, process is elevated).
/// A non-elevated admin still lists S-1-5-32-544 (deny-only); elevation shows as High (S-1-16-12288) or System integrity.
pub fn admin_flags(whoami_groups: &str) -> (bool, bool) {
    let member = whoami_groups.contains("S-1-5-32-544");
    let elevated = whoami_groups.contains("S-1-16-12288") || whoami_groups.contains("S-1-16-16384");
    (member, elevated)
}

/// Runs the system `ssh-keygen` (never bundled crypto).
fn keygen(args: &[&str]) -> Result<String> {
    let mut cmd = std::process::Command::new("ssh-keygen");
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd
        .output()
        .map_err(|e| Error::Other(format!("cannot run `ssh-keygen {}` (install the OpenSSH client): {e}", args.join(" "))))?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "`ssh-keygen {}` failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Creates an ed25519 key (no passphrase: BatchMode) at `path` unless it exists; returns the `type blob` of the public key.
pub fn ensure_device_key(path: &Path) -> Result<String> {
    if !path.exists() {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| Error::Other(format!("cannot create {}: {e}", d.display())))?;
        }
        keygen(&["-q", "-t", "ed25519", "-N", "", "-C", "wooloo-peer", "-f", &path.to_string_lossy()])?;
    }
    let pubpath = format!("{}.pub", path.to_string_lossy());
    let pubtext = std::fs::read_to_string(&pubpath).map_err(|e| Error::Other(format!("cannot read {pubpath}: {e}")))?;
    key_of(&pubtext)
}

/// `type blob` from a public key file line (drops the comment).
pub fn key_of(line: &str) -> Result<String> {
    let mut it = line.split_whitespace();
    match (it.next(), it.next()) {
        (Some(t), Some(b)) if valid_key(&format!("{t} {b}")) => Ok(format!("{t} {b}")),
        _ => bad("not a public key"),
    }
}

/// `SHA256:...` fingerprint of a `type blob` key, via `ssh-keygen -lf`.
pub fn fingerprint(key: &str) -> Result<String> {
    if !valid_key(key) {
        return bad("bad key");
    }
    let tmp = std::env::temp_dir().join(format!("wooloo-fp-{}.pub", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, format!("{key}\n")).map_err(|e| Error::Other(format!("cannot write {}: {e}", tmp.display())))?;
    let r = keygen(&["-l", "-f", &tmp.to_string_lossy()]);
    let _ = std::fs::remove_file(&tmp);
    let text = r?;
    text.split_whitespace().nth(1).map(str::to_string).ok_or_else(|| Error::Other("ssh-keygen: no fingerprint".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real ed25519 wire blobs (32 zero-ish bytes of key material are enough for structure checks).
    fn key(ty: &str) -> String {
        let mut raw = (ty.len() as u32).to_be_bytes().to_vec();
        raw.extend_from_slice(ty.as_bytes());
        raw.extend_from_slice(&[0, 0, 0, 32]);
        raw.extend_from_slice(&[7u8; 32]);
        format!("{ty} {}", STANDARD.encode(raw))
    }

    fn code() -> PairCode {
        PairCode {
            target: "me@100.64.1.2".into(),
            public_key: key("ssh-ed25519"),
            host_key: key("ssh-ed25519"),
            herdr_version: "0.9.5".into(),
            session: "wooloo".into(),
            label: "Studio Mac".into(),
        }
    }

    #[test]
    fn roundtrip_and_sorted_keys() {
        let c = code();
        let s = c.encode().unwrap();
        assert!(s.starts_with(CODE_PREFIX));
        assert!(!s.contains('='));
        let json = String::from_utf8(URL_SAFE_NO_PAD.decode(&s[CODE_PREFIX.len()..]).unwrap()).unwrap();
        let order: Vec<usize> = ["\"herdrVersion\"", "\"hostKey\"", "\"label\"", "\"publicKey\"", "\"session\"", "\"target\""]
            .iter()
            .map(|k| json.find(k).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{json}");
        assert_eq!(PairCode::decode(&s).unwrap(), c);
        assert_eq!(PairCode::decode(&format!("  {s}\n")).unwrap(), c);
    }

    #[test]
    fn decode_mac_style_code_and_rejects_path_field() {
        // The code carries NO Herdr path: the Mac peer's `{herdr}` is baked into its own dispatcher.
        let mut c = code();
        c.target = "rishabh@100.101.102.103".into();
        let enc = c.encode().unwrap();
        assert_eq!(PairCode::decode(&enc).unwrap().target, "rishabh@100.101.102.103");
        let json = serde_json::json!({"target":"me@100.64.1.2","publicKey":c.public_key,"hostKey":c.host_key,"herdrVersion":"0.9.5","session":"wooloo","label":"Mac","herdrPath":"/Users/me/.local/share/wooloo/peer/herdr"});
        let raw = format!("{CODE_PREFIX}{}", URL_SAFE_NO_PAD.encode(json.to_string()));
        assert!(PairCode::decode(&raw).is_err());
    }

    #[test]
    fn decode_rejects_bad_input() {
        assert!(PairCode::decode("nope").is_err());
        assert!(PairCode::decode("wooloo-pair-v1:!!!").is_err());
        let mut c = code();
        for t in ["-oProxyCommand=x@h", "a@b@c", "me@host:0", "me@host:70000", "me@ho st", "@host", "me", &"a".repeat(300)] {
            c.target = t.into();
            assert!(c.validate().is_err(), "{t}");
        }
        c = code();
        c.public_key = "ssh-ed25519 AAAA".into();
        assert!(c.validate().is_err());
        c = code();
        c.session = "a b".into();
        assert!(c.validate().is_err());
        c = code();
        c.herdr_version = "1;rm".into();
        assert!(c.validate().is_err());
        c = code();
        c.label = "x\ny".into();
        assert!(c.validate().is_err());
        c = code();
        c.label = "x".repeat(65);
        assert!(c.validate().is_err());
        // extra keys are refused
        let body = URL_SAFE_NO_PAD.encode(r#"{"extra":1}"#);
        assert!(PairCode::decode(&format!("{CODE_PREFIX}{body}")).is_err());
    }

    #[test]
    fn targets() {
        assert_eq!(split_target("me@host").unwrap(), ("me".into(), "host".into(), 22));
        assert_eq!(split_target("me@host:2222").unwrap(), ("me".into(), "host".into(), 2222));
        assert_eq!(split_target("me@[fd7a::1]:2200").unwrap(), ("me".into(), "fd7a::1".into(), 2200));
        assert_eq!(split_target("me@[fd7a::1]").unwrap().2, 22);
    }

    #[test]
    fn ids() {
        assert_eq!(peer_id("Studio Mac!"), "studio-mac");
        assert_eq!(peer_id("???"), "device");
        assert_eq!(peer_id(&"a".repeat(50)).len(), 32);
        assert_eq!(code().peer_id(), "studio-mac");
    }

    #[test]
    fn authorized_keys() {
        let k = key("ssh-ed25519");
        let l = authorized_keys_line("C:\\ProgramData\\wooloo\\peer-dispatch.ps1", "studio-mac", None, &k).unwrap();
        assert_eq!(
            l,
            format!("command=\"powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File C:/ProgramData/wooloo/peer-dispatch.ps1 -Peer studio-mac\",restrict {k}")
        );
        // spaces get quoted with \" inside command="..."
        let l = authorized_keys_line("C:\\Users\\A B\\p.ps1", "x", Some("C:\\h\\herdr.exe"), &k).unwrap();
        assert!(l.contains("-File \\\"C:/Users/A B/p.ps1\\\" -Peer x -Herdr C:/h/herdr.exe\",restrict "), "{l}");
        assert!(authorized_keys_line("p", "Bad Id", None, &k).is_err());
        assert!(authorized_keys_line("p", "x", None, "ssh-ed25519 AAAA").is_err());
        assert!(authorized_keys_line("p\n", "x", None, &k).is_err());
    }

    #[test]
    fn merge() {
        let k = key("ssh-ed25519");
        let l = authorized_keys_line("C:/w/peer-dispatch.ps1", "mac", None, &k).unwrap();
        let (t, changed) = merge_authorized_keys("", "mac", &l);
        assert!(changed && t == format!("{l}\r\n"));
        // idempotent
        assert!(!merge_authorized_keys(&t, "mac", &l).1);
        // re-pairing replaces the old entry for that peer
        let k2 = key("ecdsa-sha2-nistp256");
        let l2 = authorized_keys_line("C:/w/peer-dispatch.ps1", "mac", None, &k2).unwrap();
        let (t2, changed) = merge_authorized_keys(&t, "mac", &l2);
        assert!(changed && t2.contains(&k2) && !t2.contains(&k));
        // other lines are kept; an existing unrestricted entry for the same key is left alone
        let other = "ssh-ed25519 AAAAother me@x";
        let (t3, _) = merge_authorized_keys(&format!("{other}\r\n"), "mac", &l);
        assert!(t3.starts_with(other));
        let unrestricted = format!("{k}");
        let (t4, changed) = merge_authorized_keys(&format!("{unrestricted}\r\n"), "mac", &l);
        assert!(!changed && t4.starts_with(&unrestricted));
    }

    #[test]
    fn ssh_files() {
        let k = key("ssh-ed25519");
        assert_eq!(known_hosts_line("mac", &k), format!("wooloo-peer-mac {k}\n"));
        let c = ssh_conf("mac", "me@100.64.1.2:2222", "C:\\u\\key", "C:\\u\\mac.known_hosts").unwrap();
        assert!(c.starts_with("Host wooloo-peer-mac\n    HostName 100.64.1.2\n    User me\n    Port 2222\n    IdentityFile C:/u/key\n"));
        assert!(c.contains("StrictHostKeyChecking yes"));
        assert_eq!(with_include("Host a\n"), format!("{INCLUDE_LINE}\nHost a\n"));
        let once = with_include("Host a\n");
        assert_eq!(with_include(&once), once);
    }

    const ED_BLOB: &str = "AAAAC3NzaC1lZDI1NTE5AAAAICL2jZ5OHmGEQ+JCAFHyZFZq9g9k+e44/DsZhPfT6/d4";
    const RSA_BLOB: &str = "AAAAB3NzaC1yc2EAAAADAQABAAAACDEyMzQ1Njc4";

    #[test]
    fn keyscan_parses_real_lines() {
        let want = format!("ssh-ed25519 {ED_BLOB}");
        assert_eq!(parse_keyscan(&format!("127.0.0.1 ssh-ed25519 {ED_BLOB}\n")), Some(want.clone()));
        // comment lines before/after, CRLF, [host]:port, trailing comment
        let out = format!("# 127.0.0.1:22 SSH-2.0-OpenSSH_for_Windows_9.5\r\n[127.0.0.1]:2222 ssh-ed25519 {ED_BLOB} extra\r\n# done\r\n");
        assert_eq!(parse_keyscan(&out), Some(want.clone()));
        // no host column
        assert_eq!(parse_keyscan(&format!("ssh-ed25519 {ED_BLOB}")), Some(want.clone()));
        // rsa line first, ed25519 second
        let mixed = format!("127.0.0.1 ssh-rsa {RSA_BLOB}\n127.0.0.1 ssh-ed25519 {ED_BLOB}\n");
        assert_eq!(parse_keyscan(&mixed), Some(want));
    }

    #[test]
    fn keyscan_rejects_empty_rsa_only_and_garbage() {
        assert_eq!(parse_keyscan(""), None);
        assert_eq!(parse_keyscan("# 127.0.0.1:22 SSH-2.0-OpenSSH_9.5\n"), None);
        assert_eq!(parse_keyscan(&format!("127.0.0.1 ssh-rsa {RSA_BLOB}\n")), None);
        assert_eq!(parse_keyscan("127.0.0.1 ssh-ed25519 AAAA\n"), None); // blob not a real key
        assert_eq!(parse_keyscan("127.0.0.1 ssh-ed25519\n"), None);
    }

    #[test]
    fn host_key_order_keyscan_then_file() {
        use std::cell::RefCell;
        let good = format!("ssh-ed25519 {ED_BLOB}");
        let calls = RefCell::new(Vec::<String>::new());
        // first address works: later addresses and the file are never touched
        let r = resolve_host_key(
            |h| { calls.borrow_mut().push(h.into()); Ok(good.clone()) },
            || panic!("file must not be read"),
            "F",
        );
        assert_eq!(r.unwrap(), good);
        assert_eq!(*calls.borrow(), vec!["127.0.0.1"]);
        // first fails, ::1 works
        calls.borrow_mut().clear();
        let r = resolve_host_key(
            |h| { calls.borrow_mut().push(h.into()); if h == "::1" { Ok(good.clone()) } else { Err("exit 255".into()) } },
            || panic!("file must not be read"),
            "F",
        );
        assert_eq!(r.unwrap(), good);
        assert_eq!(*calls.borrow(), vec!["127.0.0.1", "::1"]);
        // all keyscans fail: file is the fallback
        calls.borrow_mut().clear();
        let r = resolve_host_key(|h| { calls.borrow_mut().push(h.into()); Err("x".into()) }, || Ok(good.clone()), "F");
        assert_eq!(r.unwrap(), good);
        assert_eq!(*calls.borrow(), vec!["127.0.0.1", "::1", "localhost"]);
    }

    #[test]
    fn host_key_error_names_every_attempt_and_fix() {
        let e = resolve_host_key(
            |h| Err(format!("exit 255: connect to host {h} port 22: Connection refused")),
            || Err(Attempt { what: String::new(), err: "Access is denied. (os error 5)".into() }),
            "C:\\ProgramData\\ssh\\ssh_host_ed25519_key.pub",
        )
        .unwrap_err();
        for s in ["ssh-keyscan -t ed25519 127.0.0.1", "ssh-keyscan -t ed25519 ::1", "ssh-keyscan -t ed25519 localhost", "Connection refused",
            "read C:\\ProgramData\\ssh\\ssh_host_ed25519_key.pub", "Access is denied. (os error 5)", "Start-Service sshd", "administrator"] {
            assert!(e.contains(s), "missing {s:?} in {e}");
        }
    }

    #[test]
    fn elevated_script_quotes_and_acl() {
        let line = "command=\"powershell -File C:/w/p.ps1 -Peer a\",restrict ssh-ed25519 AAAA it's";
        let s = elevated_commands("C:\\ProgramData\\ssh\\administrators_authorized_keys", line);
        assert!(s.contains("$p = 'C:\\ProgramData\\ssh\\administrators_authorized_keys'"));
        assert!(s.contains("it''s"));
        assert!(s.contains("Add-Content -Path $p -Value '"));
        assert!(s.contains("icacls $p /inheritance:r /grant 'SYSTEM:F' /grant 'Administrators:F'"));
    }

    #[test]
    fn admin_flags_distinguish_member_from_elevated() {
        let member_only = "BUILTIN\\Administrators Alias S-1-5-32-544 Group used for deny only\nMandatory Label\\Medium Mandatory Level Label S-1-16-8192";
        assert_eq!(admin_flags(member_only), (true, false));
        let elevated = "BUILTIN\\Administrators Alias S-1-5-32-544 Enabled group\nMandatory Label\\High Mandatory Level Label S-1-16-12288";
        assert_eq!(admin_flags(elevated), (true, true));
        assert_eq!(admin_flags("Everyone S-1-1-0\nMandatory Label\\Medium Mandatory Level Label S-1-16-8192"), (false, false));
    }
}
