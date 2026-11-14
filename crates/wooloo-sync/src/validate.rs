//! Allow-list validation (section 4). Anything not explicitly allowed is invalid.
//!
//! Patterns are matched by hand (no regex dependency) and, unlike a regex `$`,
//! never accept a trailing newline.

use serde_json::Value;

/// Kind of document key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// `space:<lowercase uuid>`
    Space,
    /// `unitcat:<lowercased Space name>`
    Unitcat,
}

pub const SPACE_PREFIX: &str = "space:";
pub const UNITCAT_PREFIX: &str = "unitcat:";

/// Every field a `space:` key may carry.
pub const SPACE_FIELDS: [&str; 9] = [
    "name",
    "category",
    "color",
    "order",
    "notes",
    "pinned_tabs",
    "host",
    "herdr_id",
    "deleted",
];

const NOTES_MAX_BYTES: usize = 65_536;
const PINNED_TABS_MAX: usize = 64;

fn is_control(c: char) -> bool {
    (c as u32) < 0x20 || c as u32 == 0x7f
}

/// "Text": a string of `lo..=hi` Unicode scalar values, none of them a control
/// character (U+0000-U+001F, U+007F).
pub fn is_text(s: &str, lo: usize, hi: usize) -> bool {
    let mut n = 0usize;
    for c in s.chars() {
        if is_control(c) {
            return false;
        }
        n += 1;
        if n > hi {
            return false;
        }
    }
    n >= lo
}

fn value_is_text(v: &Value, lo: usize, hi: usize) -> bool {
    matches!(v, Value::String(s) if is_text(s, lo, hi))
}

fn null_or_text(v: &Value, lo: usize, hi: usize) -> bool {
    v.is_null() || value_is_text(v, lo, hi)
}

/// Node id: 1-64 chars of `[a-z0-9-]` (section 2.1).
pub fn is_node_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Herdr workspace id: 1-64 chars of `[A-Za-z0-9_.:-]`.
pub fn is_herdr_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
}

fn is_lower_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
}

/// Lowercase 8-4-4-4-12 hex UUID.
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    b.iter().enumerate().all(|(i, &c)| match i {
        8 | 13 | 18 | 23 => c == b'-',
        _ => is_lower_hex(c),
    })
}

/// `#rrggbb`, lowercase hex.
pub fn is_color(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7 && b[0] == b'#' && b[1..].iter().all(|&c| is_lower_hex(c))
}

fn is_notes(s: &str) -> bool {
    s.len() <= NOTES_MAX_BYTES
        && !s
            .chars()
            .any(|c| is_control(c) && !matches!(c, '\n' | '\r' | '\t'))
}

pub fn key_kind(key: &str) -> Option<KeyKind> {
    if let Some(rest) = key.strip_prefix(SPACE_PREFIX) {
        if is_uuid(rest) {
            return Some(KeyKind::Space);
        }
    }
    if let Some(rest) = key.strip_prefix(UNITCAT_PREFIX) {
        if is_text(rest, 1, 200) {
            return Some(KeyKind::Unitcat);
        }
    }
    None
}

/// Is `value` allowed for `field` of a key of kind `kind`?
pub fn valid_value(kind: KeyKind, field: &str, value: &Value) -> bool {
    match kind {
        KeyKind::Space => match field {
            "name" => value_is_text(value, 1, 200),
            "category" => null_or_text(value, 1, 64),
            "color" => value.is_null() || matches!(value, Value::String(s) if is_color(s)),
            // serde_json cannot hold NaN or infinities, so every Number is finite.
            "order" => value.is_number(),
            "notes" => value.is_null() || matches!(value, Value::String(s) if is_notes(s)),
            "pinned_tabs" => match value {
                Value::Null => true,
                Value::Array(items) => {
                    items.len() <= PINNED_TABS_MAX && items.iter().all(|t| value_is_text(t, 1, 200))
                }
                _ => false,
            },
            "host" => matches!(value, Value::String(s) if is_node_id(s)),
            "herdr_id" => matches!(value, Value::String(s) if is_herdr_id(s)),
            "deleted" => value.is_boolean(),
            _ => false,
        },
        KeyKind::Unitcat => field == "category" && null_or_text(value, 1, 64),
    }
}

/// Full op check: key kind, writer node id, then the field's value.
pub fn valid_op(key: &str, field: &str, value: &Value, node: &str) -> bool {
    match key_kind(key) {
        Some(kind) => is_node_id(node) && valid_value(kind, field, value),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn notes_limit_is_in_bytes() {
        let ok = "a".repeat(NOTES_MAX_BYTES);
        let too_long = "a".repeat(NOTES_MAX_BYTES + 1);
        assert!(valid_value(KeyKind::Space, "notes", &json!(ok)));
        assert!(!valid_value(KeyKind::Space, "notes", &json!(too_long)));
        // 2-byte chars: 32768 of them is exactly the limit.
        let two = "é".repeat(NOTES_MAX_BYTES / 2);
        assert!(valid_value(KeyKind::Space, "notes", &json!(two)));
        let two_over = "é".repeat(NOTES_MAX_BYTES / 2 + 1);
        assert!(!valid_value(KeyKind::Space, "notes", &json!(two_over)));
    }

    #[test]
    fn patterns_reject_trailing_newline() {
        assert!(is_node_id("mac-3f9a21c0"));
        assert!(!is_node_id("mac-3f9a21c0\n"));
        assert!(!is_herdr_id("w1\n"));
        assert!(!is_color("#aabbcc\n"));
        assert!(!is_uuid("6f1c7a52-3b0e-4d8e-9a51-2a8f1c0d9e1\n"));
        assert!(is_uuid("6f1c7a52-3b0e-4d8e-9a51-2a8f1c0d9e11"));
        assert!(!is_text("a\n", 1, 200));
    }

    #[test]
    fn every_space_field_has_a_rule() {
        for f in SPACE_FIELDS {
            let sample = match f {
                "name" | "category" | "notes" => json!("x"),
                "color" => json!("#000000"),
                "order" => json!(1),
                "pinned_tabs" => json!(["x"]),
                "host" => json!("mac-1"),
                "herdr_id" => json!("w1"),
                "deleted" => json!(false),
                _ => unreachable!(),
            };
            assert!(valid_value(KeyKind::Space, f, &sample), "{f}");
        }
    }
}
