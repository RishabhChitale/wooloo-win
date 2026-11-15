//! Finding the Herdr session a local machine should use when none is configured.
//!
//! Herdr (src/session.rs, v0.9.5): the default session lives at the unnamed root (`<config>\herdr.sock`),
//! named sessions at `<config>\sessions\<name>\herdr.sock`. `herdr session list --json` reports each
//! session's `name`, `default` and `running`. Everything here that decides anything is pure so it can be
//! tested without IO; only `run_session_list` and `list_herdr_pipes` touch the system.
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub name: String,
    pub is_default: bool,
    pub running: bool,
}

#[derive(Debug, Deserialize)]
struct RawEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    running: bool,
}

/// Parses `herdr session list --json`. Accepts a bare array or an object holding an array
/// (`sessions` preferred, else the first array field), since the envelope is not pinned down.
pub fn parse_session_list(json: &str) -> Option<Vec<SessionEntry>> {
    let v: serde_json::Value = serde_json::from_str(json.trim()).ok()?;
    let arr = match &v {
        serde_json::Value::Array(a) => a.clone(),
        serde_json::Value::Object(o) => {
            let pick = o.get("sessions").filter(|x| x.is_array()).or_else(|| o.values().find(|x| x.is_array()));
            // One level deeper, e.g. {"result": {"sessions": [...]}}.
            let pick = pick.cloned().or_else(|| {
                o.values().find_map(|x| x.as_object().and_then(|i| i.get("sessions")).filter(|s| s.is_array()).cloned())
            })?;
            pick.as_array()?.clone()
        }
        _ => return None,
    };
    let mut out = Vec::new();
    for item in arr {
        let r: RawEntry = serde_json::from_value(item).ok()?;
        let is_default = r.default || r.name.as_deref().map_or(true, |n| n == "default");
        out.push(SessionEntry { name: r.name.unwrap_or_else(|| "default".into()), is_default, running: r.running });
    }
    Some(out)
}

/// Sessions implied by pipe names (`herdr.sock` pipes only, not `herdr-client.sock`): a root
/// `...\herdr\herdr.sock` is the default session, `...\herdr\sessions\<n>\herdr.sock` is `<n>`.
pub fn sessions_from_pipes(pipes: &[String]) -> Vec<SessionEntry> {
    let mut out: Vec<SessionEntry> = Vec::new();
    for p in pipes {
        let norm = p.replace('/', "\\");
        let low = norm.to_ascii_lowercase();
        if !low.ends_with("herdr.sock") || low.ends_with("-client.sock") {
            continue;
        }
        let parts: Vec<&str> = norm.split('\\').filter(|s| !s.is_empty()).collect();
        let e = match parts.iter().rposition(|s| s.eq_ignore_ascii_case("sessions")) {
            Some(i) if i + 2 < parts.len() => SessionEntry { name: parts[i + 1].to_string(), is_default: false, running: true },
            _ => SessionEntry { name: "default".into(), is_default: true, running: true },
        };
        if !out.iter().any(|x| x.name == e.name) {
            out.push(e);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Use this session (`None` = the default/unnamed root).
    Use(Option<String>),
    /// Several named sessions are running and none is the default: the user must choose.
    Ambiguous(Vec<String>),
    /// Nothing running was found: fall back to the default endpoint (the normal "no server" failure).
    Fallback,
}

/// Default session if running, else the only running one, else ask.
pub fn decide(entries: &[SessionEntry]) -> Decision {
    let running: Vec<&SessionEntry> = entries.iter().filter(|e| e.running).collect();
    if running.iter().any(|e| e.is_default) {
        return Decision::Use(None);
    }
    match running.len() {
        0 => Decision::Fallback,
        1 => Decision::Use(Some(running[0].name.clone())),
        _ => Decision::Ambiguous(running.iter().map(|e| e.name.clone()).collect()),
    }
}

/// Failure text for an ambiguous choice.
pub fn ambiguous_reason(names: &[String]) -> (String, String) {
    let first = names.first().map(String::as_str).unwrap_or("main");
    (
        format!("found sessions: {}; choose one", names.join(", ")),
        format!("set session = \"{first}\" for this machine (machines.toml, or the machine's session setting)"),
    )
}

/// `herdr session list --json` with no console window; None if herdr is missing, slow or fails.
pub async fn run_session_list() -> Option<Vec<SessionEntry>> {
    let mut c = tokio::process::Command::new("herdr");
    c.args(["session", "list", "--json"]).stdin(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let out = tokio::time::timeout(Duration::from_secs(4), c.output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    parse_session_list(&String::from_utf8_lossy(&out.stdout))
}

/// Names under `\\.\pipe\` that mention herdr (Windows only; empty elsewhere).
pub fn list_herdr_pipes() -> Vec<String> {
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        fn walk(dir: &std::path::Path, depth: u32, out: &mut Vec<String>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                let s = p.to_string_lossy().to_string();
                if s.to_ascii_lowercase().contains("herdr") {
                    out.push(s.trim_start_matches(r"\\.\pipe\").to_string());
                }
                if depth < 8 && e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    walk(&p, depth + 1, out);
                }
            }
        }
        walk(std::path::Path::new(r"\\.\pipe\"), 0, &mut out);
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Discovery for a local machine: CLI first, pipe enumeration as the fallback.
pub async fn discover() -> Decision {
    match run_session_list().await {
        Some(e) => decide(&e),
        None => decide(&sessions_from_pipes(&list_herdr_pipes())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, d: bool, r: bool) -> SessionEntry {
        SessionEntry { name: name.into(), is_default: d, running: r }
    }

    #[test]
    fn parses_array_and_envelopes() {
        let a = r#"[{"name":"default","default":true,"running":true,"socket_path":"x"},{"name":"b","default":false,"running":false}]"#;
        let want = vec![e("default", true, true), e("b", false, false)];
        assert_eq!(parse_session_list(a), Some(want.clone()));
        assert_eq!(parse_session_list(&format!(r#"{{"sessions":{a}}}"#)), Some(want.clone()));
        assert_eq!(parse_session_list(&format!(r#"{{"result":{{"sessions":{a}}}}}"#)), Some(want));
        assert_eq!(parse_session_list("not json"), None);
        assert_eq!(parse_session_list(r#"{"x":1}"#), None);
    }

    #[test]
    fn default_wins_then_only_running_then_ask() {
        assert_eq!(decide(&[e("default", true, true), e("a", false, true)]), Decision::Use(None));
        assert_eq!(decide(&[e("default", true, false), e("a", false, true)]), Decision::Use(Some("a".into())));
        assert_eq!(decide(&[e("default", true, false), e("a", false, false)]), Decision::Fallback);
        assert_eq!(decide(&[e("default", true, false), e("a", false, true), e("b", false, true)]), Decision::Ambiguous(vec!["a".into(), "b".into()]));
        assert_eq!(decide(&[]), Decision::Fallback);
    }

    #[test]
    fn ambiguous_text() {
        let (r, h) = ambiguous_reason(&["a".into(), "b".into()]);
        assert_eq!(r, "found sessions: a, b; choose one");
        assert!(h.contains("session = \"a\""));
    }

    #[test]
    fn pipes_to_sessions() {
        let p = vec![
            r"C:\Users\me\AppData\Roaming\herdr\herdr.sock".to_string(),
            r"C:\Users\me\AppData\Roaming\herdr\herdr-client.sock".to_string(),
            r"C:\Users\me\AppData\Roaming\herdr\sessions\work\herdr.sock".to_string(),
            r"C:\Users\me\AppData\Roaming\herdr\sessions\work\herdr-client.sock".to_string(),
            r"unrelated".to_string(),
        ];
        assert_eq!(sessions_from_pipes(&p), vec![e("default", true, true), e("work", false, true)]);
        assert_eq!(decide(&sessions_from_pipes(&p[2..4])), Decision::Use(Some("work".into())));
    }
}
