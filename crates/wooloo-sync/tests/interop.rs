//! Cross-implementation interop with the Swift app (wooloo `PeerSyncInteropTests`).
//!
//! Both sides run the same scripted scenario (`fixtures/interop-scenario.json`) and must produce
//! byte-identical wire transcripts. Transcript format: one wire line per line, prefixed "> "
//! (initiator to responder) or "< " (responder to initiator).
//!
//! Normal run: assert our transcript equals `fixtures/interop-rust.transcript` and `fixtures/interop-swift.transcript`
//! (a missing fixture is skipped with a note). `WOOLOO_INTEROP_REGENERATE=1` writes `interop-rust.transcript`
//! instead and prints it between INTEROP-TRANSCRIPT markers. See docs/sync-protocol.md ("Interop fixtures").

use serde_json::{json, Value};
use std::io::Cursor;
use std::path::PathBuf;
use wooloo_sync::{exchange, Register, Replica};

const SCENARIO: &str = include_str!("fixtures/interop-scenario.json");

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn regs(v: &Value) -> Vec<Register> {
    v.as_array().expect("register list").iter().map(|r| Register::from_json(r).expect("register")).collect()
}

fn lines(bytes: &[u8]) -> Vec<String> {
    std::str::from_utf8(bytes).unwrap().lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

struct Run {
    to_responder: Vec<String>,
    to_initiator: Vec<String>,
    ini_final: Vec<String>,
    resp_final: Vec<String>,
}

fn canon(rs: &[Register]) -> Vec<String> {
    rs.iter().map(|r| r.to_json().to_string()).collect()
}

fn run() -> Run {
    let s: Value = serde_json::from_str(SCENARIO).unwrap();
    let now = s["now"].as_u64().unwrap();
    let herdr = s["herdr_version"].as_str().unwrap();
    let ini_node = s["initiator_node"].as_str().unwrap();
    let resp_node = s["responder_node"].as_str().unwrap();

    let mut ini = Replica::new(ini_node);
    for r in regs(&s["initiator"]) {
        ini.insert_raw(r);
    }
    let mut resp = Replica::new(resp_node);
    for r in regs(&s["responder"]) {
        resp.insert_raw(r);
    }
    let sent = ini.sorted_regs();

    let mut stream = Vec::new();
    exchange::write_msg(&mut stream, &exchange::hello_with(ini_node, Some(herdr))).unwrap();
    for r in &sent {
        exchange::write_msg(&mut stream, &r.to_wire()).unwrap();
    }
    exchange::write_msg(&mut stream, &json!({"t": "end", "count": sent.len() as u64})).unwrap();

    let mut resp_out = Vec::new();
    exchange::respond_with(&mut Cursor::new(stream.clone()), &mut resp_out, resp_node, Some(herdr), |_, ops, malformed| {
        Ok(resp.absorb(ops, malformed, now))
    })
    .expect("respond");
    let mut ini_out = Vec::new();
    let got = exchange::initiate_with(&mut Cursor::new(resp_out.clone()), &mut ini_out, ini_node, Some(herdr), &sent).expect("initiate");
    assert_eq!(ini_out, stream, "initiate must write exactly the initiator stream");
    for r in got.reply {
        ini.apply(r, now);
    }
    Run {
        to_responder: lines(&stream),
        to_initiator: lines(&resp_out),
        ini_final: canon(&ini.sorted_regs()),
        resp_final: canon(&resp.sorted_regs()),
    }
}

fn transcript(r: &Run) -> String {
    let mut out = String::new();
    for l in &r.to_responder {
        out.push_str(&format!("> {l}\n"));
    }
    for l in &r.to_initiator {
        out.push_str(&format!("< {l}\n"));
    }
    out
}

fn fixture(name: &str) -> Option<String> {
    std::fs::read_to_string(fixture_path(name)).ok()
}

#[test]
fn scenario_converges() {
    let r = run();
    assert_eq!(r.ini_final, r.resp_final);
    assert!(!r.resp_final.is_empty());
}

#[test]
fn rust_transcript_matches_fixture() {
    let t = transcript(&run());
    if std::env::var("WOOLOO_INTEROP_REGENERATE").as_deref() == Ok("1") {
        std::fs::write(fixture_path("interop-rust.transcript"), &t).unwrap();
        println!("INTEROP-TRANSCRIPT-BEGIN rust\n{t}INTEROP-TRANSCRIPT-END");
        return;
    }
    match fixture("interop-rust.transcript") {
        Some(want) => assert_eq!(t, want, "Rust wire output drifted from the committed transcript"),
        None => {
            println!("INTEROP-TRANSCRIPT-BEGIN rust\n{t}INTEROP-TRANSCRIPT-END");
            eprintln!("interop-rust.transcript not committed yet; transcript printed");
        }
    }
}

/// The Swift transcript, replayed through the Rust responder and initiator.
#[test]
fn rust_consumes_swift_transcript() {
    let Some(text) = fixture("interop-swift.transcript") else {
        eprintln!("interop-swift.transcript not committed yet; skipping");
        return;
    };
    let own = run();
    let ini: Vec<String> = text.lines().filter_map(|l| l.strip_prefix("> ").map(str::to_owned)).collect();
    let resp: Vec<String> = text.lines().filter_map(|l| l.strip_prefix("< ").map(str::to_owned)).collect();
    assert_eq!(ini, own.to_responder, "initiator stream bytes");
    assert_eq!(resp, own.to_initiator, "responder reply bytes");

    let s: Value = serde_json::from_str(SCENARIO).unwrap();
    let now = s["now"].as_u64().unwrap();
    let herdr = s["herdr_version"].as_str().unwrap();
    let (ini_node, resp_node) = (s["initiator_node"].as_str().unwrap(), s["responder_node"].as_str().unwrap());

    let mut replica = Replica::new(resp_node);
    for r in regs(&s["responder"]) {
        replica.insert_raw(r);
    }
    let input: String = ini.iter().map(|l| format!("{l}\n")).collect();
    let mut out = Vec::new();
    exchange::respond_with(&mut Cursor::new(input.into_bytes()), &mut out, resp_node, Some(herdr), |_, ops, malformed| {
        Ok(replica.absorb(ops, malformed, now))
    })
    .expect("respond over swift input");
    assert_eq!(lines(&out), resp, "responder output over the Swift stream");
    assert_eq!(canon(&replica.sorted_regs()), own.resp_final);

    let mut ini_replica = Replica::new(ini_node);
    for r in regs(&s["initiator"]) {
        ini_replica.insert_raw(r);
    }
    let reply: String = resp.iter().map(|l| format!("{l}\n")).collect();
    let got = exchange::initiate_with(&mut Cursor::new(reply.into_bytes()), &mut Vec::new(), ini_node, Some(herdr), &ini_replica.sorted_regs())
        .expect("initiate over swift reply");
    for r in got.reply {
        ini_replica.apply(r, now);
    }
    assert_eq!(canon(&ini_replica.sorted_regs()), own.ini_final);
}
