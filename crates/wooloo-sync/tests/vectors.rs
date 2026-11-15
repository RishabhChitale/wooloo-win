//! Checks the crate against docs/sync-vectors.json (the shared vectors every
//! implementation must reproduce), plus byte-level exchange round trips.

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Cursor;
use wooloo_sync::canonical::canonical;
use wooloo_sync::{exchange, state, unitcat, validate};
use wooloo_sync::{BindKind, Error, Hlc, Outcome, Register, Rejection, Replica, MAX_FUTURE_MS};

const VECTORS: &str = include_str!("../../../docs/sync-vectors.json");
const T0: u64 = 1763112000000;
const U1: &str = "6f1c7a52-3b0e-4d8e-9a51-2a8f1c0d9e11";
const MAC: &str = "mac-3f9a21c0";
const WIN: &str = "win-07be55d1";

fn vectors() -> Value {
    serde_json::from_str(VECTORS).expect("sync-vectors.json parses")
}

fn section<'a>(v: &'a Value, name: &str) -> &'a Vec<Value> {
    v[name].as_array().unwrap_or_else(|| panic!("missing section {name}"))
}

fn name(c: &Value) -> &str {
    c["name"].as_str().unwrap_or("?")
}

fn hlc(v: &Value) -> Hlc {
    Hlc::from_json(v).expect("hlc")
}

fn reg(v: &Value) -> Register {
    Register::from_json(v).expect("register")
}

fn regs(v: &Value) -> Vec<Register> {
    v.as_array().expect("register list").iter().map(reg).collect()
}

fn load(node: &str, v: &Value) -> Replica {
    let mut r = Replica::new(node);
    for g in regs(v) {
        r.insert_raw(g);
    }
    r
}

fn as_json(rs: &[Register]) -> Value {
    Value::Array(rs.iter().map(Register::to_json).collect())
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array()
        .expect("string list")
        .iter()
        .map(|s| s.as_str().expect("string").to_owned())
        .collect()
}

fn u(v: &Value) -> u64 {
    v.as_u64().expect("unsigned integer")
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "wooloo-sync-test-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn header() {
    let v = vectors();
    assert_eq!(v["format"], json!(1));
    assert_eq!(u(&v["max_future_ms"]), MAX_FUTURE_MS);
}

#[test]
fn hlc_vectors() {
    let v = vectors();
    for c in section(&v, "hlc") {
        let mut h = hlc(&c["start"]);
        for s in c["steps"].as_array().unwrap() {
            match s["op"].as_str().unwrap() {
                "tick" => {
                    h.tick(u(&s["now"]));
                }
                "observe" => h.observe(hlc(&s["remote"])),
                other => panic!("{}: unknown step {other}", name(c)),
            }
            assert_eq!(h, hlc(&s["expect"]), "{}", name(c));
        }
    }
}

#[test]
fn canonical_vectors() {
    let v = vectors();
    for c in section(&v, "canonical") {
        assert_eq!(canonical(&c["value"]), c["text"].as_str().unwrap(), "{}", c["text"]);
    }
}

#[test]
fn compare_vectors() {
    let v = vectors();
    for c in section(&v, "compare") {
        let (a, b) = (reg(&c["a"]), reg(&c["b"]));
        let (want, back) = match c["winner"].as_str().unwrap() {
            "a" => (std::cmp::Ordering::Greater, std::cmp::Ordering::Less),
            "b" => (std::cmp::Ordering::Less, std::cmp::Ordering::Greater),
            _ => (std::cmp::Ordering::Equal, std::cmp::Ordering::Equal),
        };
        assert_eq!(a.lww_cmp(&b), want, "{}", name(c));
        assert_eq!(b.lww_cmp(&a), back, "{} (reversed)", name(c));
    }
}

#[test]
fn validate_vectors() {
    let v = vectors();
    for c in section(&v, "validate") {
        let got = Register::from_json(&c["op"]).map(|r| r.is_valid()).unwrap_or(false);
        assert_eq!(got, c["valid"].as_bool().unwrap(), "{}", name(c));
    }
}

#[test]
fn merge_vectors() {
    let v = vectors();
    for c in section(&v, "merge") {
        let now = u(&c["now"]);
        let ops = regs(&c["ops"]);
        for (order, list) in [("forward", ops.clone()), ("reversed", ops.iter().rev().cloned().collect())] {
            let mut r = Replica::new("ref-node");
            let rejected = list.into_iter().filter(|o| r.apply(o.clone(), now).is_rejected()).count();
            assert_eq!(as_json(&r.sorted_regs()), c["expect_registers"], "{} {order}", name(c));
            assert_eq!(rejected as u64, u(&c["expect_rejected"]), "{} {order}", name(c));
            assert_eq!(r.clock, hlc(&c["expect_clock"]), "{} {order}", name(c));
            assert_eq!(r.live_spaces(), strings(&c["expect_live_spaces"]), "{} {order}", name(c));
        }
    }
}

#[test]
fn exchange_vectors() {
    let v = vectors();
    for c in section(&v, "exchange") {
        let now = u(&c["now"]);
        let mut a = load(WIN, &c["initiator"]);
        let mut b = load(MAC, &c["responder"]);
        let ab = b.absorb(a.sorted_regs(), 0, now);
        assert_eq!(as_json(&ab.reply), c["expect_reply"], "{}", name(c));
        assert_eq!(ab.accepted, u(&c["expect_accepted"]), "{}", name(c));
        assert_eq!(ab.rejected, u(&c["expect_rejected"]), "{}", name(c));
        for r in ab.reply {
            a.apply(r, now);
        }
        assert_eq!(as_json(&a.sorted_regs()), c["expect_initiator"], "{}", name(c));
        assert_eq!(as_json(&b.sorted_regs()), c["expect_responder"], "{}", name(c));
    }
}

/// The same vectors through the real wire functions: hand-built initiator
/// bytes -> respond -> initiate, which must write exactly those bytes.
#[test]
fn exchange_vectors_over_the_wire() {
    let v = vectors();
    for c in section(&v, "exchange") {
        let now = u(&c["now"]);
        let a = load(WIN, &c["initiator"]);
        let mut b = load(MAC, &c["responder"]);
        let sent = a.sorted_regs();

        let mut init_bytes = Vec::new();
        exchange::write_msg(&mut init_bytes, &exchange::hello(WIN)).unwrap();
        for r in &sent {
            exchange::write_msg(&mut init_bytes, &r.to_wire()).unwrap();
        }
        exchange::write_msg(&mut init_bytes, &json!({"t": "end", "count": sent.len() as u64})).unwrap();

        let mut resp_bytes = Vec::new();
        let responded = exchange::respond(&mut Cursor::new(init_bytes.clone()), &mut resp_bytes, MAC, |peer, ops, malformed| {
            assert_eq!(peer, WIN);
            Ok(b.absorb(ops, malformed, now))
        })
        .unwrap_or_else(|e| panic!("{}: respond failed: {e}", name(c)));
        assert_eq!(responded.peer, WIN);
        assert_eq!(responded.received, sent.len() as u64);
        assert_eq!(responded.malformed, 0);

        let mut init_out = Vec::new();
        let got = exchange::initiate(&mut Cursor::new(resp_bytes), &mut init_out, WIN, &sent)
            .unwrap_or_else(|e| panic!("{}: initiate failed: {e}", name(c)));
        assert_eq!(init_out, init_bytes, "{}: initiator bytes", name(c));
        assert_eq!(got.peer, MAC);
        assert_eq!(as_json(&got.reply), c["expect_reply"], "{}", name(c));
        assert_eq!(got.malformed, 0);
        assert_eq!(got.accepted, u(&c["expect_accepted"]), "{}", name(c));
        assert_eq!(got.rejected, u(&c["expect_rejected"]), "{}", name(c));
        assert_eq!(as_json(&b.sorted_regs()), c["expect_responder"], "{}", name(c));
    }
}

fn last_line(out: &[u8]) -> Map<String, Value> {
    let text = std::str::from_utf8(out).unwrap();
    let line = text.lines().last().expect("some output");
    serde_json::from_str::<Value>(line).unwrap().as_object().unwrap().clone()
}

#[test]
fn responder_refusals_send_an_error_and_skip_the_apply() {
    let hello_line = |v: Value| {
        let mut b = Vec::new();
        exchange::write_msg(&mut b, &v).unwrap();
        b
    };
    let same = hello_line(exchange::hello(MAC));
    let proto = hello_line(json!({"t": "hello", "proto": 2, "node": WIN}));
    let mut truncated = hello_line(exchange::hello(WIN));
    let op = json!({"t": "op", "key": format!("space:{U1}"), "field": "name", "value": "A",
                    "hlc": {"wall": T0, "counter": 0}, "node": WIN});
    exchange::write_msg(&mut truncated, &op).unwrap();
    for (label, input) in [("same node", same), ("protocol", proto), ("truncated", truncated)] {
        let mut out = Vec::new();
        let mut called = false;
        let r = exchange::respond(&mut Cursor::new(input), &mut out, MAC, |_, _, _| {
            called = true;
            unreachable!()
        });
        assert!(matches!(r, Err(Error::Protocol(_))), "{label}: {r:?}");
        assert!(!called, "{label}");
        let m = last_line(&out);
        assert_eq!(m.get("t"), Some(&json!("error")), "{label}");
    }
}

#[test]
fn initiator_reports_a_peer_error_as_remote() {
    let mut out = Vec::new();
    let r = exchange::initiate(&mut Cursor::new(b"{\"t\":\"error\",\"msg\":\"nope\"}\n".to_vec()), &mut out, WIN, &[]);
    assert!(matches!(r, Err(Error::Remote(ref m)) if m == "nope"), "{r:?}");
}

#[test]
fn binding_vectors() {
    let v = vectors();
    for c in section(&v, "binding") {
        let node = c["node"].as_str().unwrap();
        let r = load(node, &c["registers"]);
        let ws: Vec<(String, String)> = c["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| (w["herdr_id"].as_str().unwrap().to_owned(), w["label"].as_str().unwrap().to_owned()))
            .collect();
        let expect = c["expect"].as_array().unwrap();
        let kind = |k: BindKind| match k {
            BindKind::Bound => "bound",
            BindKind::Rebound => "rebound",
            BindKind::Minted => "minted",
        };

        let plan = r.plan_binding(&ws);
        assert_eq!(plan.bindings.len(), expect.len(), "{}", name(c));
        for (b, e) in plan.bindings.iter().zip(expect) {
            assert_eq!(b.herdr_id, e["herdr_id"].as_str().unwrap(), "{}", name(c));
            assert_eq!(b.label, e["label"].as_str().unwrap(), "{}", name(c));
            assert_eq!(kind(b.kind), e["kind"].as_str().unwrap(), "{}", name(c));
            assert_eq!(b.space.as_deref(), e["space"].as_str(), "{}", name(c));
        }
        assert_eq!(plan.orphans, strings(&c["expect_orphans"]), "{}", name(c));

        let mut host = r.clone();
        let done = host.bind_host(&ws, T0).unwrap();
        assert_eq!(done.orphans, plan.orphans, "{}", name(c));
        for (b, e) in done.bindings.iter().zip(expect) {
            assert_eq!(kind(b.kind), e["kind"].as_str().unwrap(), "{}", name(c));
            let space = b.space.as_deref().expect("bind_host sets every space");
            match e["space"].as_str() {
                Some(s) => assert_eq!(space, s, "{}", name(c)),
                None => assert!(validate::is_uuid(space), "{}: minted {space}", name(c)),
            }
            if b.kind != BindKind::Rebound {
                assert_eq!(host.lookup(node, &b.herdr_id).as_deref(), Some(space), "{}", name(c));
            }
        }
    }
}

#[test]
fn unitcat_import_vectors() {
    let v = vectors();
    for c in section(&v, "unitcat_import") {
        let mut r = Replica::new(MAC);
        let mut snap = c["snapshot"].as_object().cloned().unwrap();
        let n = unitcat::import(&mut r, &mut snap, c["file"].as_object(), T0).unwrap();
        let ops: Vec<Value> = r
            .sorted_regs()
            .iter()
            .map(|g| json!({"key": g.key, "field": g.field, "value": g.value}))
            .collect();
        assert_eq!(Value::Array(ops), c["expect_ops"], "{}", name(c));
        assert_eq!(n, c["expect_ops"].as_array().unwrap().len(), "{}", name(c));
        assert_eq!(Value::Object(snap), c["expect_snapshot"], "{}", name(c));
    }
}

#[test]
fn unitcat_export_vectors() {
    let v = vectors();
    for c in section(&v, "unitcat_export") {
        let r = load(MAC, &c["registers"]);
        let want: BTreeMap<String, String> = c["expect"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        assert_eq!(unitcat::clean(&r.unitcats()), want, "{}", name(c));
    }
}

#[test]
fn apply_outcomes_and_delete_restore() {
    let key = format!("space:{U1}");
    let mut r = Replica::new(MAC);
    let op = reg(&json!({"key": key, "field": "name", "value": "A", "hlc": {"wall": T0, "counter": 0}, "node": WIN}));
    assert_eq!(r.apply(op.clone(), T0), Outcome::Stored);
    assert_eq!(r.apply(op.clone(), T0), Outcome::Duplicate);
    let mut old = op.clone();
    old.hlc = Hlc::new(T0 - 1, 0);
    old.value = json!("Z");
    assert_eq!(r.apply(old, T0), Outcome::Ignored);
    let mut future = op.clone();
    future.hlc = Hlc::new(T0 + MAX_FUTURE_MS + 1, 0);
    let clock = r.clock;
    assert_eq!(r.apply(future, T0), Outcome::Rejected(Rejection::Future));
    assert_eq!(r.clock, clock);
    assert_eq!(r.val(&key, "name"), &json!("A"));
    assert_eq!(r.live_spaces(), vec![U1.to_owned()]);

    r.delete_space(U1, T0).unwrap();
    assert!(r.live_spaces().is_empty());
    let undo = reg(&json!({"key": key, "field": "deleted", "value": false, "hlc": {"wall": T0 - 5, "counter": 0}, "node": WIN}));
    assert_eq!(r.apply(undo, T0), Outcome::Ignored);
    assert!(r.live_spaces().is_empty());
    r.restore_space(U1, T0).unwrap();
    assert_eq!(r.live_spaces(), vec![U1.to_owned()]);
}

#[test]
fn state_update_saves_and_releases_the_lock() {
    let dir = temp_dir("state");
    let path = dir.join("state.json");
    let node = state::update(&path, |s| {
        s.replica.set("unitcat:study central", "category", json!("school"), T0)?;
        Ok(s.replica.node.clone())
    })
    .unwrap();
    assert!(validate::is_node_id(&node));
    assert!(!dir.join("state.json.lock").exists());
    let s = state::load(&path).unwrap();
    assert_eq!(s.replica.node, node);
    assert_eq!(s.replica.val("unitcat:study central", "category"), &json!("school"));
    // A failing update saves nothing.
    let r: wooloo_sync::Result<()> = state::update(&path, |s| {
        s.replica.set("unitcat:x", "category", json!("finance"), T0)?;
        Err(Error::Invalid("stop".into()))
    });
    assert!(r.is_err());
    assert!(state::load(&path).unwrap().replica.get("unitcat:x", "category").is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unitcat_export_writes_python_text_and_skips_unsafe_targets() {
    let dir = temp_dir("unitcat");
    let path = dir.join("unit-space-categories.json");
    let mut r = Replica::new(MAC);
    r.set("unitcat:finance explorer", "category", json!("finance"), T0).unwrap();
    let mut snap = Map::new();
    assert!(unitcat::export(&r, &mut snap, &path).unwrap());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, "{\n \"finance explorer\": \"finance\"\n}");
    assert_eq!(Value::Object(snap.clone()), json!({"finance explorer": "finance"}));
    // Unchanged: not rewritten, and re-importing writes no ops.
    assert!(!unitcat::export(&r, &mut snap, &path).unwrap());
    let file = unitcat::read_file(&path);
    assert_eq!(unitcat::import(&mut r, &mut snap, file.as_ref(), T0).unwrap(), 0);
    // Missing directory: skipped.
    let gone = dir.join("missing").join("cats.json");
    assert!(!unitcat::export(&r, &mut snap, &gone).unwrap());
    assert!(!gone.exists());
    // A file that is not a JSON object is never overwritten.
    std::fs::write(&path, "[1]").unwrap();
    assert!(!unitcat::export(&r, &mut snap, &path).unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[1]");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hello_vectors() {
    let v = vectors();
    let cases = section(&v, "hello");
    assert!(!cases.is_empty());
    for c in cases {
        let line = c["line"].as_str().expect("line");
        let own = c["own"].as_str().expect("own");
        let m: Map<String, Value> = serde_json::from_str(line).expect("hello line parses");
        let got = exchange::parse_hello(&m, own);
        assert_eq!(got.is_ok(), c["expect_ok"].as_bool().unwrap(), "{}: {got:?}", name(c));
        let want_hv = c["expect_herdr_version"].as_str();
        if let Ok(h) = &got {
            assert_eq!(Some(h.node.as_str()), c["expect_peer"].as_str(), "{}", name(c));
            assert_eq!(h.herdr_version.as_deref(), want_hv, "{}", name(c));
            assert_eq!(exchange::check_hello(&m, own).unwrap(), h.node, "{}", name(c));
        }
        if c["wire"].as_bool().unwrap() {
            let peer = c["expect_peer"].as_str().expect("wire case has a peer");
            let mut b = Vec::new();
            exchange::write_msg(&mut b, &exchange::hello_with(peer, want_hv)).unwrap();
            assert_eq!(std::str::from_utf8(&b).unwrap(), format!("{line}\n"), "{}", name(c));
            if want_hv.is_none() {
                let mut b0 = Vec::new();
                exchange::write_msg(&mut b0, &exchange::hello(peer)).unwrap();
                assert_eq!(b0, b, "{}", name(c));
            }
        }
    }
}
