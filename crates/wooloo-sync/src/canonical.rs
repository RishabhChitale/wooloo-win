//! Canonical JSON text used as the last LWW tie-break (section 2.3).
//!
//! Compact, object keys sorted by UTF-8 bytes, strings escaping only `"`, `\`
//! and characters below U+0020 (`\n \t \r \b \f` short forms, others as
//! lowercase `\u00xx`), `/`, DEL and non-ASCII raw, numbers as serde_json
//! prints them (shortest round-trip form).

use serde_json::Value;
use std::fmt::Write as _;

pub fn canonical(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, &mut out);
    out
}

fn write_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => {
            let _ = write!(out, "{n}");
        }
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            // Never rely on Map iteration order (preserve_order may be enabled).
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (i, (k, val)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(k, out);
                out.push(':');
                write_value(val, out);
            }
            out.push('}');
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::canonical;
    use serde_json::json;

    #[test]
    fn sorts_keys_and_escapes_minimally() {
        let v = json!({"b": [1, "x/y"], "a": "\u{1}\u{7f}é"});
        assert_eq!(canonical(&v), "{\"a\":\"\\u0001\u{7f}é\",\"b\":[1,\"x/y\"]}");
    }
}
