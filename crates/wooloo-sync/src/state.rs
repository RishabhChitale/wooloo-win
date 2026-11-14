//! The state file (section 6): `~/.config/wooloo-sync/state.json`, guarded by
//! `state.json.lock`, written through `state.json.tmp` + rename.

use crate::doc::{Register, Replica};
use crate::hlc::Hlc;
use crate::validate::is_node_id;
use crate::{with_suffix, Error, Result, FORMAT};
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// A lock file older than this is stale and removed.
pub const LOCK_STALE: Duration = Duration::from_secs(30);
/// How long [`Lock::acquire`] waits for a held lock before giving up.
pub const LOCK_WAIT: Duration = Duration::from_secs(10);
const LOCK_RETRY: Duration = Duration::from_millis(50);

/// Everything in the state file.
#[derive(Clone, Debug)]
pub struct State {
    pub replica: Replica,
    /// `local.unitcat_snapshot`: never sent to peers.
    pub unitcat_snapshot: Map<String, Value>,
}

/// `HOME` if set and non-empty, else `USERPROFILE` (Windows).
pub fn home_dir() -> Option<PathBuf> {
    for var in ["HOME", "USERPROFILE"] {
        if let Some(v) = std::env::var_os(var) {
            if !v.is_empty() {
                return Some(PathBuf::from(v));
            }
        }
    }
    None
}

/// `WOOLOO_SYNC_STATE`, else `~/.config/wooloo-sync/state.json`.
pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WOOLOO_SYNC_STATE") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    home_dir().map(|h| h.join(".config").join("wooloo-sync").join("state.json"))
}

/// A fresh node id: `mac-`, `win-` or `linux-` plus 8 random lowercase hex digits.
pub fn new_node_id() -> String {
    let os = if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(windows) {
        "win"
    } else {
        "linux"
    };
    let hex = uuid::Uuid::new_v4().simple().to_string().to_ascii_lowercase();
    format!("{os}-{}", &hex[..8])
}

impl State {
    /// An empty state with a new node id.
    pub fn fresh() -> State {
        State {
            replica: Replica::new(new_node_id()),
            unitcat_snapshot: Map::new(),
        }
    }

    /// Parse a state file's JSON. Registers that do not parse or are not on
    /// the allow-list are dropped; if a pair appears twice the LWW winner is kept.
    pub fn from_json(v: &Value) -> Result<State> {
        let o = v
            .as_object()
            .ok_or_else(|| Error::Invalid("state file is not a JSON object".into()))?;
        match o.get("format").and_then(Value::as_u64) {
            Some(FORMAT) => {}
            other => {
                return Err(Error::Invalid(format!(
                    "state file format {} is not supported (expected {FORMAT})",
                    other.map_or_else(|| "missing".to_string(), |f| f.to_string())
                )))
            }
        }
        let node = o
            .get("node")
            .and_then(Value::as_str)
            .filter(|n| is_node_id(n))
            .ok_or_else(|| Error::Invalid("state file node id missing or malformed".into()))?;
        let mut replica = Replica::new(node);
        if let Some(c) = o.get("clock").and_then(Hlc::from_json) {
            replica.clock = c;
        }
        if let Some(Value::Array(regs)) = o.get("registers") {
            for rv in regs {
                let Ok(reg) = Register::from_json(rv) else { continue };
                if !reg.is_valid() {
                    continue;
                }
                let keep = match replica.get(&reg.key, &reg.field) {
                    Some(cur) => reg.lww_cmp(cur) == Ordering::Greater,
                    None => true,
                };
                if keep {
                    // Raises the clock to the register's hlc (section 6).
                    replica.insert_raw(reg);
                }
            }
        }
        let unitcat_snapshot = o
            .get("local")
            .and_then(|l| l.get("unitcat_snapshot"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        Ok(State {
            replica,
            unitcat_snapshot,
        })
    }

    pub fn to_json(&self) -> Value {
        let regs: Vec<Value> = self.replica.sorted_regs().iter().map(Register::to_json).collect();
        json!({
            "format": FORMAT,
            "node": self.replica.node,
            "clock": self.replica.clock.to_json(),
            "registers": regs,
            "local": { "unitcat_snapshot": Value::Object(self.unitcat_snapshot.clone()) },
        })
    }
}

/// Read the state file. A missing file gives [`State::fresh`]; an unreadable,
/// corrupt or unsupported one is an error (it is never silently replaced).
pub fn load(path: &Path) -> Result<State> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(State::fresh()),
        Err(e) => return Err(e.into()),
    };
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
    State::from_json(&v)
}

/// Write the state file through `state.json.tmp` + rename. Callers hold the lock.
pub fn save(path: &Path, state: &State) -> Result<()> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir)?;
        }
    }
    let mut text = serde_json::to_string_pretty(&state.to_json())
        .map_err(|e| Error::Invalid(format!("cannot serialize state: {e}")))?;
    text.push('\n');
    let tmp = with_suffix(path, ".tmp");
    fs::write(&tmp, text)?;
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// The `state.json.lock` file; removed on drop.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
}

impl Lock {
    /// Create the lock file exclusively, removing it first if it is stale.
    /// Waits up to [`LOCK_WAIT`] for another holder, then fails with `Busy`.
    pub fn acquire(state_path: &Path) -> Result<Lock> {
        let path = with_suffix(state_path, ".lock");
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                fs::create_dir_all(dir)?;
            }
        }
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = writeln!(f, "{}", std::process::id());
                    return Ok(Lock { path });
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                    if is_stale(&path) {
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                }
                // Windows reports a lock file that is being deleted as access denied.
                Err(e) if e.kind() == ErrorKind::PermissionDenied && path.exists() => {}
                Err(e) => return Err(e.into()),
            }
            if Instant::now() >= deadline {
                return Err(Error::Busy(format!("{} is held by another process", path.display())));
            }
            thread::sleep(LOCK_RETRY);
        }
    }
}

fn is_stale(lock: &Path) -> bool {
    let Ok(meta) = fs::metadata(lock) else { return false };
    let Ok(modified) = meta.modified() else { return false };
    // A modification time in the future counts as fresh.
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age > LOCK_STALE)
        .unwrap_or(false)
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Lock, re-read, run `f`, and save only if `f` succeeded. The lock is
/// released on return either way.
pub fn update<T>(path: &Path, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
    let _lock = Lock::acquire(path)?;
    let mut state = load(path)?;
    let out = f(&mut state)?;
    save(path, &state)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_are_valid() {
        for _ in 0..20 {
            let n = new_node_id();
            assert!(is_node_id(&n), "{n}");
            assert_eq!(n.rsplit('-').next().unwrap().len(), 8);
        }
    }

    #[test]
    fn rejects_wrong_format_and_bad_node() {
        assert!(State::from_json(&json!({"format": 2, "node": "mac-1"})).is_err());
        assert!(State::from_json(&json!({"format": 1, "node": "Mac 1"})).is_err());
        assert!(State::from_json(&json!([])).is_err());
        let s = State::from_json(&json!({"format": 1, "node": "mac-1"})).unwrap();
        assert!(s.replica.is_empty());
        assert_eq!(s.replica.clock, Hlc::ZERO);
    }

    #[test]
    fn load_drops_invalid_keeps_winner_and_raises_clock() {
        let u = "space:6f1c7a52-3b0e-4d8e-9a51-2a8f1c0d9e11";
        let v = json!({
            "format": 1,
            "node": "mac-1",
            "clock": {"wall": 5, "counter": 0},
            "registers": [
                {"key": u, "field": "name", "value": "new", "hlc": {"wall": 9, "counter": 1}, "node": "win-1"},
                {"key": u, "field": "name", "value": "old", "hlc": {"wall": 8, "counter": 0}, "node": "win-1"},
                {"key": u, "field": "bogus", "value": 1, "hlc": {"wall": 99, "counter": 0}, "node": "win-1"},
                {"key": u, "field": "name"}
            ],
            "local": {"unitcat_snapshot": {"a": "school"}}
        });
        let s = State::from_json(&v).unwrap();
        assert_eq!(s.replica.len(), 1);
        assert_eq!(s.replica.val(u, "name"), &json!("new"));
        assert_eq!(s.replica.clock, Hlc::new(9, 1));
        assert_eq!(s.unitcat_snapshot.get("a"), Some(&json!("school")));
    }
}
