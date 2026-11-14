//! The replicated document: LWW registers, merge, Space helpers and the host
//! binding pass (sections 2-5).

use crate::canonical::canonical;
use crate::hlc::Hlc;
use crate::validate::{self, SPACE_PREFIX, UNITCAT_PREFIX};
use crate::{Error, Result, MAX_FUTURE_MS};
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};

static NULL: Value = Value::Null;

/// One field of one key (section 2). An op on the wire is exactly a register.
#[derive(Clone, Debug, PartialEq)]
pub struct Register {
    pub key: String,
    pub field: String,
    pub value: Value,
    pub hlc: Hlc,
    pub node: String,
}

impl Register {
    /// Parse the register members of a JSON object. Unknown members (such as
    /// the wire's `"t"`) are ignored. This checks shape only; [`Register::is_valid`]
    /// checks the allow-list.
    pub fn from_json(v: &Value) -> Result<Register> {
        let o = v
            .as_object()
            .ok_or_else(|| Error::Invalid("register is not an object".into()))?;
        let text = |name: &str| -> Result<String> {
            o.get(name)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| Error::Invalid(format!("register member `{name}` missing or not a string")))
        };
        let value = o
            .get("value")
            .cloned()
            .ok_or_else(|| Error::Invalid("register member `value` missing".into()))?;
        let hlc = o
            .get("hlc")
            .and_then(Hlc::from_json)
            .ok_or_else(|| Error::Invalid("register member `hlc` missing or malformed".into()))?;
        Ok(Register {
            key: text("key")?,
            field: text("field")?,
            value,
            hlc,
            node: text("node")?,
        })
    }

    pub fn to_json(&self) -> Value {
        json!({
            "key": self.key,
            "field": self.field,
            "value": self.value,
            "hlc": self.hlc.to_json(),
            "node": self.node,
        })
    }

    /// The register as a wire message: `{"t":"op", <register members>}`.
    pub fn to_wire(&self) -> Value {
        let mut v = self.to_json();
        if let Value::Object(m) = &mut v {
            m.insert("t".into(), Value::String("op".into()));
        }
        v
    }

    /// Allow-list check (section 4).
    pub fn is_valid(&self) -> bool {
        validate::valid_op(&self.key, &self.field, &self.value, &self.node)
    }

    /// Last-writer-wins order (section 2.3): wall, counter, node bytes, then
    /// canonical value text bytes. `Greater` means `self` wins.
    pub fn lww_cmp(&self, other: &Register) -> Ordering {
        self.hlc
            .cmp(&other.hlc)
            .then_with(|| self.node.as_bytes().cmp(other.node.as_bytes()))
            .then_with(|| canonical(&self.value).as_bytes().cmp(canonical(&other.value).as_bytes()))
    }

    /// Byte-identical register (same pair, hlc, node and canonical value).
    pub fn same(&self, other: &Register) -> bool {
        self.key == other.key
            && self.field == other.field
            && self.hlc == other.hlc
            && self.node == other.node
            && canonical(&self.value) == canonical(&other.value)
    }

    fn pair(&self) -> (String, String) {
        (self.key.clone(), self.field.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// Not on the allow-list.
    Invalid,
    /// Wall clock more than `MAX_FUTURE_MS` ahead of the receiver (deferred).
    Future,
}

/// Result of [`Replica::apply`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Stored (no register for the pair yet, or the op wins).
    Stored,
    /// Identical to the stored register.
    Duplicate,
    /// Loses to the stored register.
    Ignored,
    Rejected(Rejection),
}

impl Outcome {
    pub fn is_rejected(self) -> bool {
        matches!(self, Outcome::Rejected(_))
    }
}

/// Result of [`Replica::absorb`] (the responder side of an exchange).
#[derive(Clone, Debug, PartialEq)]
pub struct Absorbed {
    /// Ops read that were not rejected (stored, duplicate or ignored).
    pub accepted: u64,
    /// Ops rejected by validation or the skew guard, plus malformed ops.
    pub rejected: u64,
    /// Registers to send back: those missing from, or different to, what was received.
    pub reply: Vec<Register>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindKind {
    /// A live Space already has this workspace's `herdr_id`.
    Bound,
    /// A live Space with a stale `herdr_id` and the same name takes this workspace.
    Rebound,
    /// No Space qualifies; a new UUID is minted.
    Minted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub herdr_id: String,
    pub label: String,
    pub kind: BindKind,
    /// The Space UUID (without `space:`). `None` for `Minted` in a plan; set
    /// after [`Replica::bind_host`] has minted it.
    pub space: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindPlan {
    /// One per workspace, in the order given.
    pub bindings: Vec<Binding>,
    /// Live Spaces hosted here that no workspace bound (sorted UUIDs).
    pub orphans: Vec<String>,
}

/// One machine's copy of the document.
#[derive(Clone, Debug)]
pub struct Replica {
    pub node: String,
    pub clock: Hlc,
    regs: BTreeMap<(String, String), Register>,
}

fn space_key(uuid: &str) -> String {
    format!("{SPACE_PREFIX}{uuid}")
}

impl Replica {
    pub fn new(node: impl Into<String>) -> Replica {
        Replica {
            node: node.into(),
            clock: Hlc::ZERO,
            regs: BTreeMap::new(),
        }
    }

    /// Merge one op (section 2.4).
    pub fn apply(&mut self, op: Register, now: u64) -> Outcome {
        if !op.is_valid() {
            return Outcome::Rejected(Rejection::Invalid);
        }
        if op.hlc.wall > now.saturating_add(MAX_FUTURE_MS) {
            return Outcome::Rejected(Rejection::Future);
        }
        self.clock.observe(op.hlc);
        let pair = op.pair();
        match self.regs.get(&pair) {
            None => {
                self.regs.insert(pair, op);
                Outcome::Stored
            }
            Some(cur) => match op.lww_cmp(cur) {
                Ordering::Greater => {
                    self.regs.insert(pair, op);
                    Outcome::Stored
                }
                Ordering::Equal => Outcome::Duplicate,
                Ordering::Less => Outcome::Ignored,
            },
        }
    }

    /// Local write: tick the clock and store the op. Errors if the value is not
    /// allowed (nothing is changed then, not even the clock).
    pub fn set(&mut self, key: &str, field: &str, value: Value, now: u64) -> Result<Register> {
        if !validate::valid_op(key, field, &value, &self.node) {
            return Err(Error::Invalid(format!("value not allowed for {key} {field}")));
        }
        let hlc = self.clock.tick(now);
        let op = Register {
            key: key.to_owned(),
            field: field.to_owned(),
            value,
            hlc,
            node: self.node.clone(),
        };
        // A fresh tick is strictly ahead of every stored register (they were all
        // observed), so this always stores; the skew guard does not apply to
        // our own writes.
        self.regs.insert(op.pair(), op.clone());
        Ok(op)
    }

    /// Insert a register as-is (state file load, tests) and observe its clock.
    /// It is not validated; callers validate first.
    pub fn insert_raw(&mut self, reg: Register) {
        self.clock.observe(reg.hlc);
        self.regs.insert(reg.pair(), reg);
    }

    pub fn get(&self, key: &str, field: &str) -> Option<&Register> {
        self.regs.get(&(key.to_owned(), field.to_owned()))
    }

    /// The register's value, or `null` if there is none.
    pub fn val(&self, key: &str, field: &str) -> &Value {
        self.get(key, field).map(|r| &r.value).unwrap_or(&NULL)
    }

    /// All registers sorted by `(key, field)` bytes.
    pub fn sorted_regs(&self) -> Vec<Register> {
        // BTreeMap<(String, String)> iterates in byte order of key then field.
        self.regs.values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.regs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.regs.is_empty()
    }

    fn space_uuids(&self) -> BTreeSet<&str> {
        self.regs
            .keys()
            .filter_map(|(k, _)| k.strip_prefix(SPACE_PREFIX))
            .collect()
    }

    fn is_deleted(&self, uuid: &str) -> bool {
        self.val(&space_key(uuid), "deleted") == &Value::Bool(true)
    }

    /// UUIDs of Spaces whose `deleted` register is not `true`, sorted.
    pub fn live_spaces(&self) -> Vec<String> {
        self.space_uuids()
            .into_iter()
            .filter(|u| !self.is_deleted(u))
            .map(str::to_owned)
            .collect()
    }

    /// The live Space hosted on `host` with workspace id `herdr_id` (smallest
    /// UUID if several), for clients viewing a peer's server (section 3).
    pub fn lookup(&self, host: &str, herdr_id: &str) -> Option<String> {
        self.live_spaces().into_iter().find(|u| {
            let k = space_key(u);
            self.val(&k, "host").as_str() == Some(host) && self.val(&k, "herdr_id").as_str() == Some(herdr_id)
        })
    }

    /// Mint a new Space hosted here (section 3, step 3). Returns its UUID.
    pub fn mint_space(&mut self, herdr_id: &str, name: &str, now: u64) -> Result<String> {
        if !validate::is_herdr_id(herdr_id) || !validate::is_text(name, 1, 200) {
            return Err(Error::Invalid("workspace id or name not allowed".into()));
        }
        let uuid = loop {
            let u = uuid::Uuid::new_v4().hyphenated().to_string().to_ascii_lowercase();
            if !self.space_uuids().contains(u.as_str()) {
                break u;
            }
        };
        let key = space_key(&uuid);
        let host = Value::String(self.node.clone());
        self.set(&key, "host", host, now)?;
        self.set(&key, "herdr_id", Value::String(herdr_id.to_owned()), now)?;
        self.set(&key, "name", Value::String(name.to_owned()), now)?;
        Ok(uuid)
    }

    /// Tombstone a Space and clear its notes and pinned tabs (section 5).
    pub fn delete_space(&mut self, uuid: &str, now: u64) -> Result<()> {
        let key = space_key(uuid);
        self.set(&key, "deleted", Value::Bool(true), now)?;
        self.set(&key, "notes", Value::Null, now)?;
        self.set(&key, "pinned_tabs", Value::Null, now)?;
        Ok(())
    }

    /// Explicitly restore a deleted Space (`deleted = false` with a newer hlc).
    pub fn restore_space(&mut self, uuid: &str, now: u64) -> Result<()> {
        self.set(&space_key(uuid), "deleted", Value::Bool(false), now)?;
        Ok(())
    }

    /// Non-null `unitcat:*` categories as `{name: category}` (section 7 export).
    pub fn unitcats(&self) -> Map<String, Value> {
        let mut out = Map::new();
        for ((k, f), r) in &self.regs {
            if let Some(name) = k.strip_prefix(UNITCAT_PREFIX) {
                if f == "category" {
                    if let Value::String(s) = &r.value {
                        out.insert(name.to_owned(), Value::String(s.clone()));
                    }
                }
            }
        }
        out
    }

    /// Host binding pass (section 3) for this node's workspaces `(id, label)`,
    /// in Herdr's order. Pure: writes nothing.
    pub fn plan_binding(&self, workspaces: &[(String, String)]) -> BindPlan {
        let me = self.node.as_str();
        // live_spaces is sorted, so "first match" is "smallest UUID".
        let live: Vec<String> = self
            .live_spaces()
            .into_iter()
            .filter(|u| self.val(&space_key(u), "host").as_str() == Some(me))
            .collect();
        let ids: BTreeSet<&str> = workspaces.iter().map(|(w, _)| w.as_str()).collect();
        let mut used: BTreeSet<&str> = BTreeSet::new();
        let mut bound: HashMap<usize, (BindKind, String)> = HashMap::new();

        // Step 1 over all workspaces first.
        for (i, (w, _)) in workspaces.iter().enumerate() {
            if let Some(u) = live
                .iter()
                .find(|u| !used.contains(u.as_str()) && self.val(&space_key(u), "herdr_id").as_str() == Some(w))
            {
                used.insert(u);
                bound.insert(i, (BindKind::Bound, u.clone()));
            }
        }
        // Step 2: rebind a Space whose herdr_id is stale and whose name matches.
        for (i, (_, label)) in workspaces.iter().enumerate() {
            if bound.contains_key(&i) {
                continue;
            }
            if let Some(u) = live.iter().find(|u| {
                let k = space_key(u);
                let stale = match self.val(&k, "herdr_id").as_str() {
                    Some(h) => !ids.contains(h),
                    None => true,
                };
                !used.contains(u.as_str()) && stale && self.val(&k, "name").as_str() == Some(label.as_str())
            }) {
                used.insert(u);
                bound.insert(i, (BindKind::Rebound, u.clone()));
            }
        }
        let bindings = workspaces
            .iter()
            .enumerate()
            .map(|(i, (w, label))| {
                let (kind, space) = match bound.remove(&i) {
                    Some((kind, u)) => (kind, Some(u)),
                    None => (BindKind::Minted, None),
                };
                Binding {
                    herdr_id: w.clone(),
                    label: label.clone(),
                    kind,
                    space,
                }
            })
            .collect();
        let orphans = live.iter().filter(|u| !used.contains(u.as_str())).cloned().collect();
        BindPlan { bindings, orphans }
    }

    /// Run the binding pass and write its result: `herdr_id` for rebinds, a new
    /// Space for each minted workspace. Names of bound Spaces are left to the
    /// app (Herdr renames vs. synced renames, section 3). Orphans are returned,
    /// not deleted.
    pub fn bind_host(&mut self, workspaces: &[(String, String)], now: u64) -> Result<BindPlan> {
        let mut plan = self.plan_binding(workspaces);
        for b in &mut plan.bindings {
            match b.kind {
                BindKind::Bound => {}
                BindKind::Rebound => {
                    let uuid = b.space.as_deref().unwrap_or_default();
                    self.set(&space_key(uuid), "herdr_id", Value::String(b.herdr_id.clone()), now)?;
                }
                BindKind::Minted => {
                    b.space = Some(self.mint_space(&b.herdr_id, &b.label, now)?);
                }
            }
        }
        Ok(plan)
    }

    /// Responder side of an exchange (section 8): apply the received ops and
    /// compute the reply. `malformed` counts op messages that did not parse as
    /// registers; they count as rejected.
    pub fn absorb(&mut self, received: Vec<Register>, malformed: u64, now: u64) -> Absorbed {
        let mut accepted = 0u64;
        let mut rejected = malformed;
        let mut got: HashMap<(String, String), Register> = HashMap::new();
        for op in received {
            got.insert(op.pair(), op.clone());
            if self.apply(op, now).is_rejected() {
                rejected += 1;
            } else {
                accepted += 1;
            }
        }
        let reply = self
            .regs
            .iter()
            .filter(|(pair, r)| !got.get(*pair).is_some_and(|g| g.same(r)))
            .map(|(_, r)| r.clone())
            .collect();
        Absorbed {
            accepted,
            rejected,
            reply,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: u64 = 1_763_112_000_000;

    #[test]
    fn set_rejects_disallowed_values_without_ticking() {
        let mut r = Replica::new("mac-1");
        assert!(r.set("space:nope", "name", json!("x"), T).is_err());
        assert_eq!(r.clock, Hlc::ZERO);
        assert!(r.is_empty());
    }

    #[test]
    fn mint_then_delete_then_restore() {
        let mut r = Replica::new("mac-1");
        let u = r.mint_space("w1", "Alpha", T).unwrap();
        assert!(validate::is_uuid(&u));
        assert_eq!(r.live_spaces(), vec![u.clone()]);
        assert_eq!(r.lookup("mac-1", "w1"), Some(u.clone()));
        r.set(&space_key(&u), "notes", json!("hello"), T).unwrap();
        r.delete_space(&u, T).unwrap();
        assert!(r.live_spaces().is_empty());
        assert_eq!(r.val(&space_key(&u), "notes"), &Value::Null);
        r.restore_space(&u, T).unwrap();
        assert_eq!(r.live_spaces(), vec![u]);
    }
}
