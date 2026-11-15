//! Space metadata replica and peer sync (docs/sync-protocol.md), without Tauri.
//!
//! The replica lives in one `wooloo-sync` state file in the app config dir. A sync with a
//! paired machine runs the allow-listed `wooloo-peer sync` request over the SAME paired ssh
//! channel (host alias `wooloo-peer-<id>`); the peer's forced-command dispatcher answers it by
//! running `wooloo-sync exchange`. All functions here block; call them from `spawn_blocking`.
//! Unit-state categories (sync-protocol section 7) are a Mac-side bridge and are not touched here.
use crate::{Error, Result};
use serde::Serialize;
use serde_json::{Map, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use wooloo_sync::validate::SPACE_FIELDS;
use wooloo_sync::{exchange, now_ms, state, Outcome};

/// The allow-listed request that makes the peer run its exchange responder.
pub const SYNC_REQUEST: &str = "wooloo-peer sync";
/// State file name inside the app config dir.
pub const STATE_FILE: &str = "meta-state.json";

fn e<T: std::fmt::Display>(x: T) -> Error {
    Error::Other(format!("sync: {x}"))
}

/// What one exchange did.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SyncReport {
    pub peer: String,
    pub sent: u64,
    pub received: u64,
    /// Registers that changed the local replica.
    pub stored: u64,
    pub rejected: u64,
    pub peer_accepted: u64,
    pub peer_rejected: u64,
}

impl SyncReport {
    pub fn changed(&self) -> bool {
        self.stored > 0
    }
}

/// Local replica of Space metadata, persisted at `path`.
#[derive(Clone, Debug)]
pub struct MetaStore {
    path: PathBuf,
}

/// Input to [`MetaStore::set_space_meta`].
#[derive(Clone, Debug, Default)]
pub struct SetMeta {
    /// Existing Space; or leave empty and give `herdr_id` (and `host`).
    pub uuid: Option<String>,
    pub herdr_id: Option<String>,
    /// Node id of the machine hosting the Space; default is this machine.
    pub host: Option<String>,
    /// Any of name, category, color, order, notes, pinned_tabs, deleted. `null` clears a field.
    pub fields: Map<String, Value>,
}

impl MetaStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        MetaStore { path: path.into() }
    }

    /// Store inside the app config dir.
    pub fn in_dir(dir: &Path) -> Self {
        Self::new(dir.join(STATE_FILE))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// This machine's node id (created on first use).
    pub fn node(&self) -> Result<String> {
        state::update(&self.path, |s| Ok(s.replica.node.clone())).map_err(e)
    }

    /// Every live Space as `{uuid, host, herdr_id, name, category, color, order, notes, pinned_tabs}`.
    pub fn get_meta(&self) -> Result<Value> {
        state::update(&self.path, |s| {
            let r = &s.replica;
            let mut spaces = Vec::new();
            for uuid in r.live_spaces() {
                let key = space_key(&uuid);
                let mut o = Map::new();
                o.insert("uuid".into(), Value::String(uuid));
                for f in SPACE_FIELDS.iter().filter(|f| **f != "deleted") {
                    o.insert((*f).into(), r.val(&key, f).clone());
                }
                spaces.push(Value::Object(o));
            }
            Ok(serde_json::json!({ "node": r.node, "spaces": spaces }))
        })
        .map_err(e)
    }

    /// Set fields on a Space, minting it when it is a new Space hosted on this machine.
    /// Returns the Space's uuid.
    pub fn set_space_meta(&self, req: SetMeta) -> Result<String> {
        for k in req.fields.keys() {
            if !SPACE_FIELDS.contains(&k.as_str()) || k == "host" || k == "herdr_id" {
                return Err(Error::Other(format!("field not settable: {k}")));
            }
        }
        state::update(&self.path, |s| {
            let now = now_ms();
            let r = &mut s.replica;
            let uuid = match &req.uuid {
                Some(u) => {
                    if !r.live_spaces().contains(u) {
                        return Err(wooloo_sync::Error::Invalid(format!("unknown space {u}")));
                    }
                    u.clone()
                }
                None => {
                    let hid = req
                        .herdr_id
                        .as_deref()
                        .ok_or_else(|| wooloo_sync::Error::Invalid("uuid or herdr_id required".into()))?;
                    let host = req.host.clone().unwrap_or_else(|| r.node.clone());
                    match r.lookup(&host, hid) {
                        Some(u) => u,
                        None if host == r.node => {
                            let name = req.fields.get("name").and_then(Value::as_str).unwrap_or(hid).to_owned();
                            r.mint_space(hid, &name, now)?
                        }
                        None => return Err(wooloo_sync::Error::Invalid("space not known yet; sync first".into())),
                    }
                }
            };
            let key = space_key(&uuid);
            for (f, v) in &req.fields {
                r.set(&key, f, v.clone(), now)?;
            }
            Ok(uuid)
        })
        .map_err(e)
    }

    /// Initiator over any blocking stream pair (ssh child pipes, or an in-memory pipe in tests).
    /// The state lock is not held while the stream is in use.
    pub fn sync_over<R: BufRead, W: Write>(&self, input: &mut R, output: &mut W) -> Result<SyncReport> {
        let (node, regs) = state::update(&self.path, |s| Ok((s.replica.node.clone(), s.replica.sorted_regs()))).map_err(e)?;
        let init = exchange::initiate(input, output, &node, &regs).map_err(e)?;
        let received = init.reply.len() as u64 + init.malformed;
        let (stored, rejected) = state::update(&self.path, |s| {
            if s.replica.node != node {
                return Err(wooloo_sync::Error::Invalid("node id changed during the exchange".into()));
            }
            let now = now_ms();
            let (mut stored, mut rejected) = (0u64, init.malformed);
            for reg in init.reply {
                match s.replica.apply(reg, now) {
                    Outcome::Stored => stored += 1,
                    Outcome::Rejected(_) => rejected += 1,
                    Outcome::Duplicate | Outcome::Ignored => {}
                }
            }
            Ok((stored, rejected))
        })
        .map_err(e)?;
        Ok(SyncReport {
            peer: init.peer,
            sent: regs.len() as u64,
            received,
            stored,
            rejected,
            peer_accepted: init.accepted,
            peer_rejected: init.rejected,
        })
    }

    /// Responder over any blocking stream pair. The real peer runs `wooloo-sync exchange`
    /// instead; this exists so both ends can be tested without ssh.
    pub fn respond_over<R: BufRead, W: Write>(&self, input: &mut R, output: &mut W) -> Result<SyncReport> {
        let node = self.node()?;
        let path = self.path.clone();
        let done = exchange::respond(input, output, &node, |_peer, ops, malformed| {
            state::update(&path, |s| {
                if s.replica.node != node {
                    return Err(wooloo_sync::Error::Invalid("node id changed during the exchange".into()));
                }
                Ok(s.replica.absorb(ops, malformed, now_ms()))
            })
        })
        .map_err(e)?;
        Ok(SyncReport {
            peer: done.peer,
            received: done.received,
            rejected: done.absorbed.rejected,
            peer_accepted: 0,
            peer_rejected: 0,
            sent: done.absorbed.reply.len() as u64,
            stored: 0,
        })
    }

    /// Sync with a paired machine over ssh. `host_alias` is `wooloo-peer-<id>`.
    pub fn sync_ssh(&self, host_alias: &str) -> Result<SyncReport> {
        if !valid_alias(host_alias) {
            return Err(Error::Other("sync needs a paired machine (wooloo-peer-<id>)".into()));
        }
        let mut child = Command::new("ssh")
            .args(["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "--", host_alias, SYNC_REQUEST])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|x| Error::Other(format!("cannot start ssh: {x}")))?;
        let (mut to_peer, from_peer) = match (child.stdin.take(), child.stdout.take()) {
            (Some(i), Some(o)) => (i, o),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Other("ssh pipes unavailable".into()));
            }
        };
        let mut from_peer = BufReader::new(from_peer);
        let res = self.sync_over(&mut from_peer, &mut to_peer);
        drop(to_peer);
        if res.is_err() {
            let _ = child.kill();
        }
        drop(from_peer);
        let _ = child.wait();
        res
    }
}

fn space_key(uuid: &str) -> String {
    format!("{}{uuid}", wooloo_sync::validate::SPACE_PREFIX)
}

/// `wooloo-peer-<id>` with the id charset used by pairing.
pub fn valid_alias(a: &str) -> bool {
    match a.strip_prefix("wooloo-peer-") {
        Some(id) => !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::Read;
    use std::sync::{Arc, Condvar, Mutex};

    /// One direction of an in-memory pipe: blocking reads, EOF when the writer drops.
    #[derive(Default)]
    struct Chan {
        q: Mutex<(VecDeque<u8>, bool)>,
        cv: Condvar,
    }
    struct PipeW(Arc<Chan>);
    struct PipeR(Arc<Chan>);
    impl Write for PipeW {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.q.lock().unwrap().0.extend(b);
            self.0.cv.notify_all();
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Drop for PipeW {
        fn drop(&mut self) {
            self.0.q.lock().unwrap().1 = true;
            self.0.cv.notify_all();
        }
    }
    impl Read for PipeR {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let mut g = self.0.q.lock().unwrap();
            while g.0.is_empty() && !g.1 {
                g = self.0.cv.wait(g).unwrap();
            }
            let n = out.len().min(g.0.len());
            for slot in out.iter_mut().take(n) {
                *slot = g.0.pop_front().unwrap();
            }
            Ok(n)
        }
    }
    fn pipe() -> (PipeW, PipeR) {
        let c = Arc::new(Chan::default());
        (PipeW(c.clone()), PipeR(c))
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wooloo-meta-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(STATE_FILE)
    }

    /// Run initiator `a` against responder `b` over two in-memory pipes.
    fn exchange_pair(a: &MetaStore, b: &MetaStore) -> (SyncReport, SyncReport) {
        let (a_to_b_w, a_to_b_r) = pipe();
        let (b_to_a_w, b_to_a_r) = pipe();
        let b2 = b.clone();
        let th = std::thread::spawn(move || {
            let mut r = BufReader::new(a_to_b_r);
            let mut w = b_to_a_w;
            b2.respond_over(&mut r, &mut w)
        });
        let mut r = BufReader::new(b_to_a_r);
        let mut w = a_to_b_w;
        let ra = a.sync_over(&mut r, &mut w).unwrap();
        drop(w);
        (ra, th.join().unwrap().unwrap())
    }

    fn fields(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn alias_check() {
        assert!(valid_alias("wooloo-peer-studio-mac"));
        assert!(!valid_alias("wooloo-peer-"));
        assert!(!valid_alias("studio-mac"));
        assert!(!valid_alias("wooloo-peer-a b"));
        assert!(!valid_alias("wooloo-peer-a;b"));
    }

    #[test]
    fn set_and_get_meta_persists() {
        let p = tmp("persist");
        let a = MetaStore::new(&p);
        let uuid = a
            .set_space_meta(SetMeta {
                herdr_id: Some("w_1".into()),
                fields: fields(serde_json::json!({"name": "Study", "category": "school"})),
                ..Default::default()
            })
            .unwrap();
        let again = MetaStore::new(&p).get_meta().unwrap();
        let sp = &again["spaces"][0];
        assert_eq!(sp["uuid"], uuid);
        assert_eq!(sp["name"], "Study");
        assert_eq!(sp["category"], "school");
        assert_eq!(sp["host"], again["node"]);
    }

    #[test]
    fn set_meta_rejects_bad_input() {
        let a = MetaStore::new(tmp("bad"));
        let bad = |f: Value| SetMeta { herdr_id: Some("w_1".into()), fields: fields(f), ..Default::default() };
        assert!(a.set_space_meta(bad(serde_json::json!({"host": "x"}))).is_err());
        assert!(a.set_space_meta(bad(serde_json::json!({"nope": 1}))).is_err());
        assert!(a.set_space_meta(SetMeta::default()).is_err());
        let unknown = SetMeta { uuid: Some("00000000-0000-4000-8000-000000000000".into()), ..Default::default() };
        assert!(a.set_space_meta(unknown).is_err());
    }

    #[test]
    fn two_replicas_converge_over_duplex_pipe() {
        let (pa, pb) = (tmp("a"), tmp("b"));
        let (a, b) = (MetaStore::new(&pa), MetaStore::new(&pb));
        let ua = a
            .set_space_meta(SetMeta {
                herdr_id: Some("w_a".into()),
                fields: fields(serde_json::json!({"name": "Mac space", "color": "#112233"})),
                ..Default::default()
            })
            .unwrap();
        let ub = b
            .set_space_meta(SetMeta {
                herdr_id: Some("w_b".into()),
                fields: fields(serde_json::json!({"name": "Windows space"})),
                ..Default::default()
            })
            .unwrap();

        let (ra, _rb) = exchange_pair(&a, &b);
        assert!(ra.changed());
        assert_eq!(ra.rejected, 0);
        assert_eq!(ra.peer, b.node().unwrap());

        let (ma, mb) = (a.get_meta().unwrap(), b.get_meta().unwrap());
        for m in [&ma, &mb] {
            let ids: Vec<&str> = m["spaces"].as_array().unwrap().iter().map(|s| s["uuid"].as_str().unwrap()).collect();
            assert!(ids.contains(&ua.as_str()) && ids.contains(&ub.as_str()));
        }
        assert_eq!(ma["spaces"], mb["spaces"]);
    }

    #[test]
    fn newer_edit_wins_and_second_sync_is_quiet() {
        let (a, b) = (MetaStore::new(tmp("a2")), MetaStore::new(tmp("b2")));
        let u = a
            .set_space_meta(SetMeta {
                herdr_id: Some("w_a".into()),
                fields: fields(serde_json::json!({"name": "One"})),
                ..Default::default()
            })
            .unwrap();
        exchange_pair(&a, &b);
        std::thread::sleep(std::time::Duration::from_millis(5));
        b.set_space_meta(SetMeta {
            uuid: Some(u.clone()),
            fields: fields(serde_json::json!({"name": "Two", "notes": "from windows"})),
            ..Default::default()
        })
        .unwrap();
        exchange_pair(&a, &b);
        let m = a.get_meta().unwrap();
        assert_eq!(m["spaces"][0]["name"], "Two");
        assert_eq!(m["spaces"][0]["notes"], "from windows");
        let (again, _) = exchange_pair(&a, &b);
        assert!(!again.changed());
    }

    #[test]
    fn deleted_space_disappears_on_peer() {
        let (a, b) = (MetaStore::new(tmp("a3")), MetaStore::new(tmp("b3")));
        let u = a
            .set_space_meta(SetMeta { herdr_id: Some("w_a".into()), ..Default::default() })
            .unwrap();
        exchange_pair(&a, &b);
        assert_eq!(b.get_meta().unwrap()["spaces"].as_array().unwrap().len(), 1);
        std::thread::sleep(std::time::Duration::from_millis(5));
        a.set_space_meta(SetMeta {
            uuid: Some(u),
            fields: fields(serde_json::json!({"deleted": true})),
            ..Default::default()
        })
        .unwrap();
        exchange_pair(&a, &b);
        assert!(b.get_meta().unwrap()["spaces"].as_array().unwrap().is_empty());
    }

    #[test]
    fn peer_error_is_reported() {
        let a = MetaStore::new(tmp("a4"));
        let (to_peer_w, _to_peer_r) = pipe();
        let (from_peer_w, from_peer_r) = pipe();
        drop(from_peer_w); // peer closes immediately: no hello
        let mut r = BufReader::new(from_peer_r);
        let mut w = to_peer_w;
        assert!(a.sync_over(&mut r, &mut w).is_err());
    }
}
