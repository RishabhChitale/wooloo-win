//! Bridge to the unit-state categories file (section 7):
//! `~/.local/state/unit-space-categories.json`, `{"<lowercased Space name>": "<category>"}`,
//! written by `unit-space-purpose.sh` with Python `json.dump(indent=1, sort_keys=True)`.

use crate::doc::Replica;
use crate::validate::{is_text, UNITCAT_PREFIX};
use crate::{with_suffix, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// `WOOLOO_SYNC_UNITCAT`, else `~/.local/state/unit-space-categories.json`.
pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WOOLOO_SYNC_UNITCAT") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    crate::state::home_dir().map(|h| h.join(".local").join("state").join("unit-space-categories.json"))
}

/// The file as a JSON object, or `None` if it is missing, unreadable, not
/// JSON or not an object (import then does nothing).
pub fn read_file(path: &Path) -> Option<Map<String, Value>> {
    let text = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<Value>(&text).ok()? {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

/// Only the entries that would be valid ops: name text 1-200, value a string
/// of text 1-64. Sorted by name bytes.
pub fn clean(m: &Map<String, Value>) -> BTreeMap<String, String> {
    m.iter()
        .filter_map(|(k, v)| match v {
            Value::String(s) if is_text(k, 1, 200) && is_text(s, 1, 64) => Some((k.clone(), s.clone())),
            _ => None,
        })
        .collect()
}

/// Pure three-way diff of the file against the snapshot: `(name, new value)`
/// for every name whose cleaned value changed, `None` meaning removed. Sorted
/// by name bytes.
pub fn diff(snapshot: &Map<String, Value>, file: &Map<String, Value>) -> Vec<(String, Option<String>)> {
    let f = clean(file);
    let s = clean(snapshot);
    let mut names: Vec<&String> = f.keys().chain(s.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|n| f.get(*n) != s.get(*n))
        .map(|n| (n.clone(), f.get(n).cloned()))
        .collect()
}

fn to_map(m: &BTreeMap<String, String>) -> Map<String, Value> {
    m.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect()
}

/// Import step. `file` is [`read_file`]'s result. Writes one `category` op per
/// changed name and sets the snapshot to the cleaned file. Returns the number
/// of ops written. With `file == None` nothing changes.
pub fn import(
    replica: &mut Replica,
    snapshot: &mut Map<String, Value>,
    file: Option<&Map<String, Value>>,
    now: u64,
) -> Result<usize> {
    let Some(file) = file else { return Ok(0) };
    let changes = diff(snapshot, file);
    for (name, value) in &changes {
        let v = value.clone().map(Value::String).unwrap_or(Value::Null);
        replica.set(&format!("{UNITCAT_PREFIX}{name}"), "category", v, now)?;
    }
    *snapshot = to_map(&clean(file));
    Ok(changes.len())
}

/// Export step: write the non-null `unitcat:*` categories to `path` if they
/// differ from the file's (cleaned) content. The snapshot is set to them only
/// once the file holds them (written now, or already equal). Skipped (returns
/// `false`, snapshot unchanged) if the file's directory does not exist, or if
/// the file exists but is unreadable or not a JSON object, so a damaged user
/// file is never overwritten. Returns whether the file was written.
pub fn export(replica: &Replica, snapshot: &mut Map<String, Value>, path: &Path) -> Result<bool> {
    let dir_exists = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.is_dir(),
        _ => true,
    };
    if !dir_exists {
        return Ok(false);
    }
    let want = clean(&replica.unitcats());
    let have = read_file(path);
    let unchanged = match &have {
        Some(m) => clean(m) == want,
        None if path.exists() => return Ok(false),
        // Missing file and nothing to write: leave it missing.
        None => want.is_empty(),
    };
    if !unchanged {
        let tmp = with_suffix(path, ".tmp");
        fs::write(&tmp, to_unit_file_text(&want))?;
        if let Err(e) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
    }
    *snapshot = to_map(&want);
    Ok(!unchanged)
}

/// Python `json.dumps(m, indent=1, sort_keys=True)` (ensure_ascii on, no
/// trailing newline) for a flat string map.
pub fn to_unit_file_text(m: &BTreeMap<String, String>) -> String {
    if m.is_empty() {
        return "{}".into();
    }
    let mut out = String::from("{");
    for (i, (k, v)) in m.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("\n ");
        py_string(k, &mut out);
        out.push_str(": ");
        py_string(v, &mut out);
    }
    out.push_str("\n}");
    out
}

/// A JSON string as Python's ensure_ascii encoder writes it.
fn py_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{:04x}", u);
                }
            }
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn matches_python_indent_one_sort_keys() {
        assert_eq!(to_unit_file_text(&BTreeMap::new()), "{}");
        assert_eq!(
            to_unit_file_text(&m(&[("study central", "school"), ("finance explorer", "finance")])),
            "{\n \"finance explorer\": \"finance\",\n \"study central\": \"school\"\n}"
        );
        // ensure_ascii: lowercase hex, surrogate pairs, DEL escaped.
        assert_eq!(
            to_unit_file_text(&m(&[("café ♡", "a\u{7f}"), ("x😀", "\"\\")])),
            "{\n \"caf\\u00e9 \\u2661\": \"a\\u007f\",\n \"x\\ud83d\\ude00\": \"\\\"\\\\\"\n}"
        );
    }

    #[test]
    fn written_text_parses_back() {
        let want = m(&[("a", "b"), ("é", "c")]);
        let v: Value = serde_json::from_str(&to_unit_file_text(&want)).unwrap();
        assert_eq!(clean(v.as_object().unwrap()), want);
    }
}
