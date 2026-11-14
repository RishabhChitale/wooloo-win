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
        if l.contains(&marker) && l.starts_with("command=") {
            continue;
        }
        if l == line {
            return (existing.to_string(), false);
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

/// Runs the system `ssh-keygen` (never bundled crypto).
fn keygen(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("ssh-keygen")
        .args(args)
        .output()
        .map_err(|e| Error::Other(format!("cannot run ssh-keygen (install the OpenSSH client): {e}")))?;
    if !out.status.success() {
        return Err(Error::Other(format!("ssh-keygen: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Creates an ed25519 key (no passphrase: BatchMode) at `path` unless it exists; returns the `type blob` of the public key.
pub fn ensure_device_key(path: &Path) -> Result<String> {
    if !path.exists() {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        keygen(&["-q", "-t", "ed25519", "-N", "", "-C", "wooloo-peer", "-f", &path.to_string_lossy()])?;
    }
    let pubtext = std::fs::read_to_string(format!("{}.pub", path.to_string_lossy()))?;
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
    std::fs::write(&tmp, format!("{key}\n"))?;
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
}
