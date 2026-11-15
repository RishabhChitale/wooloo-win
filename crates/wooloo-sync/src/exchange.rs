//! The exchange protocol (section 8): line-delimited JSON over a byte stream,
//! normally `ssh <peer> wooloo-sync exchange`.
//!
//! These functions only move messages. Locking, loading and saving the state
//! file is the caller's job (the responder does it inside the closure passed
//! to [`respond`], the initiator before and after [`initiate`]).

use crate::doc::{Absorbed, Register};
use crate::validate::is_node_id;
use crate::{Error, Result, PROTO};
use serde_json::{json, Map, Value};
use std::io::{BufRead, BufWriter, Read as _, Write};

/// Longest accepted line, without its `\n`.
pub const MAX_LINE: usize = 1 << 20;
/// Most ops accepted in one direction of one exchange.
pub const MAX_OPS: usize = 200_000;
/// Longest peer error text kept (in characters).
const MAX_ERROR_CHARS: usize = 300;

/// Read one message. EOF, an over-long line, bad JSON, a non-object or a
/// missing `t` is a protocol error; an `error` message becomes [`Error::Remote`].
pub fn read_msg<R: BufRead>(input: &mut R) -> Result<Map<String, Value>> {
    let mut buf = Vec::new();
    let n = (&mut *input)
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Err(Error::Protocol("connection closed".into()));
    }
    if buf.last() != Some(&b'\n') {
        return Err(Error::Protocol(if buf.len() > MAX_LINE {
            "line too long".into()
        } else {
            "connection closed mid-line".into()
        }));
    }
    buf.pop();
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    let v: Value = serde_json::from_slice(&buf).map_err(|e| Error::Protocol(format!("bad JSON: {e}")))?;
    let Value::Object(m) = v else {
        return Err(Error::Protocol("message is not a JSON object".into()));
    };
    match m.get("t").and_then(Value::as_str) {
        None => Err(Error::Protocol("message has no `t`".into())),
        Some("error") => {
            let msg = m.get("msg").and_then(Value::as_str).unwrap_or("(no message)");
            Err(Error::Remote(sanitize(msg)))
        }
        Some(_) => Ok(m),
    }
}

/// Control characters become `?`; at most [`MAX_ERROR_CHARS`] characters.
fn sanitize(s: &str) -> String {
    s.chars()
        .take(MAX_ERROR_CHARS)
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// Write one message and its `\n` (not flushed).
pub fn write_msg<W: Write>(output: &mut W, msg: &Value) -> Result<()> {
    let mut line = serde_json::to_vec(msg).map_err(|e| Error::Invalid(format!("cannot serialize message: {e}")))?;
    if line.len() > MAX_LINE {
        return Err(Error::Invalid("message longer than the 1 MiB line limit".into()));
    }
    line.push(b'\n');
    output.write_all(&line)?;
    Ok(())
}

/// Longest `herdr_version` accepted in a `hello` (bytes).
pub const HERDR_VERSION_MAX: usize = 32;

/// A `herdr_version` value (section 8): 1 to 32 bytes of ASCII letters,
/// digits or `+.-`. The same rule as pairing.
pub fn is_herdr_version(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= HERDR_VERSION_MAX
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
}

/// `{"t":"hello","proto":1,"node":...}`
pub fn hello(node: &str) -> Value {
    hello_with(node, None)
}

/// `hello` with an optional `herdr_version`. A version that fails
/// [`is_herdr_version`] is left out, never sent.
pub fn hello_with(node: &str, herdr_version: Option<&str>) -> Value {
    let mut v = json!({"t": "hello", "proto": PROTO, "node": node});
    if let (Some(hv), Value::Object(m)) = (herdr_version.filter(|h| is_herdr_version(h)), &mut v) {
        m.insert("herdr_version".into(), Value::String(hv.to_owned()));
    }
    v
}

/// What a peer's `hello` said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    pub node: String,
    /// The peer's Herdr version; `None` when absent or not a valid version
    /// (an invalid value is ignored, never an error).
    pub herdr_version: Option<String>,
}

/// Check a peer's `hello` and return its node id.
pub fn check_hello(m: &Map<String, Value>, own: &str) -> Result<String> {
    parse_hello(m, own).map(|h| h.node)
}

/// Check a peer's `hello` and return its node id and Herdr version.
pub fn parse_hello(m: &Map<String, Value>, own: &str) -> Result<Hello> {
    let t = m.get("t").and_then(Value::as_str).unwrap_or("");
    if t != "hello" {
        return Err(Error::Protocol(format!("expected hello, got `{}`", sanitize(t))));
    }
    match m.get("proto") {
        Some(p) if p.as_u64() == Some(PROTO) => {}
        other => {
            let theirs = other.map_or_else(|| "none".to_string(), |p| sanitize(&p.to_string()));
            return Err(Error::Protocol(format!(
                "peer speaks protocol {theirs}, this side speaks {PROTO}"
            )));
        }
    }
    let node = m
        .get("node")
        .and_then(Value::as_str)
        .filter(|n| is_node_id(n))
        .ok_or_else(|| Error::Protocol("peer node id missing or malformed".into()))?;
    if node == own {
        return Err(Error::Protocol(format!(
            "peer has the same node id ({node}) as this machine; two machines must never share a node id"
        )));
    }
    let herdr_version = m
        .get("herdr_version")
        .and_then(Value::as_str)
        .filter(|h| is_herdr_version(h))
        .map(str::to_owned);
    Ok(Hello {
        node: node.to_owned(),
        herdr_version,
    })
}

/// Read `op` messages up to a matching `end`. Returns the ops that parsed, the
/// number that did not (malformed), and the `end` message.
pub fn read_ops<R: BufRead>(input: &mut R) -> Result<(Vec<Register>, u64, Map<String, Value>)> {
    let mut ops = Vec::new();
    let mut malformed = 0u64;
    loop {
        let m = read_msg(input)?;
        let t = m.get("t").and_then(Value::as_str).unwrap_or("").to_owned();
        match t.as_str() {
            "op" => {
                if ops.len() as u64 + malformed >= MAX_OPS as u64 {
                    return Err(Error::Protocol(format!("refusing more than {MAX_OPS} ops")));
                }
                match Register::from_json(&Value::Object(m)) {
                    Ok(r) => ops.push(r),
                    Err(_) => malformed += 1,
                }
            }
            "end" => {
                let read = ops.len() as u64 + malformed;
                return match m.get("count").and_then(Value::as_u64) {
                    Some(c) if c == read => Ok((ops, malformed, m)),
                    Some(c) => Err(Error::Protocol(format!("end says {c} ops but {read} were read"))),
                    None => Err(Error::Protocol("end without a count".into())),
                };
            }
            other => return Err(Error::Protocol(format!("unexpected message `{}`", sanitize(other)))),
        }
    }
}

/// Tell the peer why this side is giving up, then flush. Errors that came
/// from the peer are not echoed. Local details that could carry file paths
/// (i/o, busy, invalid data) are replaced by a generic text (section 9).
/// Write failures are ignored: the stream may already be gone.
pub fn send_error<W: Write>(output: &mut W, err: &Error) {
    let msg = match err {
        Error::Remote(_) => return,
        Error::Protocol(m) => format!("protocol error: {m}"),
        Error::Busy(_) => "the peer's sync state is busy; try again".to_string(),
        Error::Io(_) => "i/o error on the peer".to_string(),
        Error::Invalid(_) => "the peer could not process the exchange".to_string(),
    };
    let _ = write_msg(output, &json!({"t": "error", "msg": msg}));
    let _ = output.flush();
}

/// What the responder did.
#[derive(Clone, Debug, PartialEq)]
pub struct Responded {
    pub peer: String,
    /// The `herdr_version` from the peer's hello, if it sent a valid one.
    pub peer_herdr_version: Option<String>,
    /// Ops received that parsed.
    pub received: u64,
    /// Op messages that did not parse.
    pub malformed: u64,
    pub absorbed: Absorbed,
}

/// Responder side. After a complete `end`, calls `f(peer, ops, malformed)`,
/// which must apply and save (and return the reply); only then is the reply
/// sent. On any error an `error` message is sent (unless the peer sent one).
pub fn respond<R, W, F>(input: &mut R, output: &mut W, node: &str, f: F) -> Result<Responded>
where
    R: BufRead,
    W: Write,
    F: FnOnce(&str, Vec<Register>, u64) -> Result<Absorbed>,
{
    respond_with(input, output, node, None, f)
}

/// [`respond`], with this side's Herdr version in its hello.
pub fn respond_with<R, W, F>(
    input: &mut R,
    output: &mut W,
    node: &str,
    herdr_version: Option<&str>,
    f: F,
) -> Result<Responded>
where
    R: BufRead,
    W: Write,
    F: FnOnce(&str, Vec<Register>, u64) -> Result<Absorbed>,
{
    let mut out = BufWriter::new(output);
    match respond_inner(input, &mut out, node, herdr_version, f) {
        Ok(r) => {
            out.flush()?;
            Ok(r)
        }
        Err(e) => {
            send_error(&mut out, &e);
            Err(e)
        }
    }
}

fn respond_inner<R, W, F>(
    input: &mut R,
    out: &mut W,
    node: &str,
    herdr_version: Option<&str>,
    f: F,
) -> Result<Responded>
where
    R: BufRead,
    W: Write,
    F: FnOnce(&str, Vec<Register>, u64) -> Result<Absorbed>,
{
    let Hello {
        node: peer,
        herdr_version: peer_herdr_version,
    } = parse_hello(&read_msg(input)?, node)?;
    write_msg(out, &hello_with(node, herdr_version))?;
    out.flush()?;
    let (ops, malformed, _end) = read_ops(input)?;
    let received = ops.len() as u64;
    let absorbed = f(&peer, ops, malformed)?;
    for r in &absorbed.reply {
        write_msg(out, &r.to_wire())?;
    }
    write_msg(
        out,
        &json!({
            "t": "end",
            "count": absorbed.reply.len() as u64,
            "accepted": absorbed.accepted,
            "rejected": absorbed.rejected,
        }),
    )?;
    out.flush()?;
    Ok(Responded {
        peer,
        peer_herdr_version,
        received,
        malformed,
        absorbed,
    })
}

/// What the initiator received.
#[derive(Clone, Debug, PartialEq)]
pub struct Initiated {
    pub peer: String,
    /// The `herdr_version` from the peer's hello, if it sent a valid one.
    pub peer_herdr_version: Option<String>,
    /// The responder's reply ops that parsed; the caller applies and saves them.
    pub reply: Vec<Register>,
    /// Reply op messages that did not parse.
    pub malformed: u64,
    /// The responder's counts of our ops (0 if it did not say).
    pub accepted: u64,
    pub rejected: u64,
}

/// Initiator side: hello, send `regs`, read the reply. Nothing is applied
/// here. On any error an `error` message is sent (unless the peer sent one).
pub fn initiate<R, W>(input: &mut R, output: &mut W, node: &str, regs: &[Register]) -> Result<Initiated>
where
    R: BufRead,
    W: Write,
{
    initiate_with(input, output, node, None, regs)
}

/// [`initiate`], with this side's Herdr version in its hello.
pub fn initiate_with<R, W>(
    input: &mut R,
    output: &mut W,
    node: &str,
    herdr_version: Option<&str>,
    regs: &[Register],
) -> Result<Initiated>
where
    R: BufRead,
    W: Write,
{
    let mut out = BufWriter::new(output);
    match initiate_inner(input, &mut out, node, herdr_version, regs) {
        Ok(r) => Ok(r),
        Err(e) => {
            send_error(&mut out, &e);
            Err(e)
        }
    }
}

fn initiate_inner<R, W>(
    input: &mut R,
    out: &mut W,
    node: &str,
    herdr_version: Option<&str>,
    regs: &[Register],
) -> Result<Initiated>
where
    R: BufRead,
    W: Write,
{
    write_msg(out, &hello_with(node, herdr_version))?;
    out.flush()?;
    let Hello {
        node: peer,
        herdr_version: peer_herdr_version,
    } = parse_hello(&read_msg(input)?, node)?;
    if regs.len() > MAX_OPS {
        return Err(Error::Protocol(format!("refusing to send more than {MAX_OPS} ops")));
    }
    if let Err(e) = send_ops(out, regs) {
        // The responder may have given up and closed with an error message
        // (for example after a refusal); prefer its reason over a broken pipe.
        if matches!(e, Error::Io(_)) {
            if let Err(remote @ Error::Remote(_)) = read_msg(input) {
                return Err(remote);
            }
        }
        return Err(e);
    }
    let (reply, malformed, end) = read_ops(input)?;
    let count = |name: &str| end.get(name).and_then(Value::as_u64).unwrap_or(0);
    Ok(Initiated {
        peer,
        peer_herdr_version,
        reply,
        malformed,
        accepted: count("accepted"),
        rejected: count("rejected"),
    })
}

fn send_ops<W: Write>(out: &mut W, regs: &[Register]) -> Result<()> {
    for r in regs {
        write_msg(out, &r.to_wire())?;
    }
    write_msg(out, &json!({"t": "end", "count": regs.len() as u64}))?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn read(s: &[u8]) -> Result<Map<String, Value>> {
        read_msg(&mut Cursor::new(s.to_vec()))
    }

    #[test]
    fn reads_lf_and_crlf_lines() {
        let mut c = Cursor::new(b"{\"t\":\"a\"}\n{\"t\":\"b\",\"x\":1}\r\n".to_vec());
        assert_eq!(read_msg(&mut c).unwrap().get("t"), Some(&json!("a")));
        assert_eq!(read_msg(&mut c).unwrap().get("x"), Some(&json!(1)));
        assert!(matches!(read_msg(&mut c), Err(Error::Protocol(m)) if m == "connection closed"));
    }

    #[test]
    fn rejects_bad_lines() {
        assert!(matches!(read(b""), Err(Error::Protocol(m)) if m == "connection closed"));
        assert!(matches!(read(b"{\"t\":\"a\"}"), Err(Error::Protocol(m)) if m == "connection closed mid-line"));
        assert!(matches!(read(b"[1]\n"), Err(Error::Protocol(_))));
        assert!(matches!(read(b"{\"x\":1}\n"), Err(Error::Protocol(_))));
        assert!(matches!(read(b"nope\n"), Err(Error::Protocol(_))));
        let mut long = vec![b' '; MAX_LINE + 10];
        long.push(b'\n');
        assert!(matches!(read(&long), Err(Error::Protocol(m)) if m == "line too long"));
        // Exactly MAX_LINE bytes before the newline is allowed.
        let mut edge = b"{\"t\":\"a\"}".to_vec();
        edge.resize(MAX_LINE, b' ');
        edge.push(b'\n');
        assert!(read(&edge).is_ok());
    }

    #[test]
    fn error_message_becomes_remote_and_is_sanitized() {
        let r = read(b"{\"t\":\"error\",\"msg\":\"bad\\u001b[31m thing\"}\n");
        assert!(matches!(r, Err(Error::Remote(m)) if m == "bad?[31m thing"));
    }

    #[test]
    fn hello_checks() {
        let h = |v: Value| v.as_object().unwrap().clone();
        assert_eq!(check_hello(&h(hello("mac-1")), "win-1").unwrap(), "mac-1");
        assert!(check_hello(&h(hello("mac-1")), "mac-1").is_err());
        assert!(check_hello(&h(json!({"t":"hello","proto":2,"node":"mac-1"})), "win-1").is_err());
        assert!(check_hello(&h(json!({"t":"hello","proto":1,"node":"Mac 1"})), "win-1").is_err());
        assert!(check_hello(&h(json!({"t":"op","proto":1,"node":"mac-1"})), "win-1").is_err());
    }

    #[test]
    fn hello_herdr_version_is_optional() {
        let h = |v: Value| v.as_object().unwrap().clone();
        let mut line = Vec::new();
        write_msg(&mut line, &hello_with("mac-1", Some("0.9.5"))).unwrap();
        assert_eq!(line, b"{\"herdr_version\":\"0.9.5\",\"node\":\"mac-1\",\"proto\":1,\"t\":\"hello\"}\n");
        // An invalid version is never sent.
        assert_eq!(hello_with("mac-1", Some("1;rm")), hello("mac-1"));
        assert_eq!(hello_with("mac-1", Some("")), hello("mac-1"));
        let got = parse_hello(&h(hello_with("mac-1", Some("0.9.5"))), "win-1").unwrap();
        assert_eq!(got.herdr_version.as_deref(), Some("0.9.5"));
        assert_eq!(parse_hello(&h(hello("mac-1")), "win-1").unwrap().herdr_version, None);
        // Any other value is ignored, not an error.
        for bad in [json!(95), json!("1;rm"), json!(""), json!("a".repeat(33)), json!(null)] {
            let m = h(json!({"t":"hello","proto":1,"node":"mac-1","herdr_version":bad}));
            assert_eq!(parse_hello(&m, "win-1").unwrap().herdr_version, None);
        }
        assert!(is_herdr_version(&"a".repeat(32)));
        assert!(is_herdr_version("1.2.3-rc.1+git.abc"));
    }

    #[test]
    fn exchange_carries_both_herdr_versions() {
        let init_hello = {
            let mut b = Vec::new();
            write_msg(&mut b, &hello_with("mac-1", Some("0.9.5"))).unwrap();
            write_msg(&mut b, &json!({"t":"end","count":0})).unwrap();
            b
        };
        let mut resp_out = Vec::new();
        let done = respond_with(&mut Cursor::new(init_hello), &mut resp_out, "win-1", Some("0.9.4"), |_, _, _| {
            Ok(Absorbed {
                accepted: 0,
                rejected: 0,
                reply: Vec::new(),
            })
        })
        .unwrap();
        assert_eq!(done.peer_herdr_version.as_deref(), Some("0.9.5"));
        let mut init_out = Vec::new();
        let got = initiate_with(&mut Cursor::new(resp_out), &mut init_out, "mac-1", Some("0.9.5"), &[]).unwrap();
        assert_eq!(got.peer, "win-1");
        assert_eq!(got.peer_herdr_version.as_deref(), Some("0.9.4"));
    }

    #[test]
    fn read_ops_counts_malformed_and_checks_end() {
        let op = r#"{"t":"op","key":"unitcat:a","field":"category","value":"x","hlc":{"wall":1,"counter":0},"node":"mac-1"}"#;
        let text = format!("{op}\n{{\"t\":\"op\",\"key\":1}}\n{{\"t\":\"end\",\"count\":2}}\n");
        let (ops, bad, _) = read_ops(&mut Cursor::new(text.into_bytes())).unwrap();
        assert_eq!((ops.len(), bad), (1, 1));
        let wrong = format!("{op}\n{{\"t\":\"end\",\"count\":2}}\n");
        assert!(read_ops(&mut Cursor::new(wrong.into_bytes())).is_err());
        let truncated = format!("{op}\n");
        assert!(read_ops(&mut Cursor::new(truncated.into_bytes())).is_err());
        let odd = "{\"t\":\"hello\"}\n".to_string();
        assert!(read_ops(&mut Cursor::new(odd.into_bytes())).is_err());
    }

    #[test]
    fn send_error_hides_local_details() {
        let mut out = Vec::new();
        send_error(&mut out, &Error::Busy("/Users/x/.config/wooloo-sync/state.json.lock".into()));
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("/Users"));
        assert!(text.ends_with('\n'));
        let mut none = Vec::new();
        send_error(&mut none, &Error::Remote("x".into()));
        assert!(none.is_empty());
    }
}
