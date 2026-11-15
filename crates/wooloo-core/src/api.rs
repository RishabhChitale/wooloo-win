//! Newline-delimited JSON socket API client and snapshot nesting.
use crate::transport::BoxStream;
use crate::{Error, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const TIMEOUT: Duration = Duration::from_secs(3);
pub const MAX_RESPONSE: usize = 8_000_000;

pub struct ApiClient {
    stream: BoxStream,
    buf: Vec<u8>,
}

impl ApiClient {
    pub fn new(stream: BoxStream) -> Self {
        ApiClient { stream, buf: Vec::new() }
    }

    /// Reads one newline-terminated line (without the newline).
    pub async fn read_line(&mut self) -> Result<Vec<u8>> {
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=i).collect();
                line.pop();
                return Ok(line);
            }
            if self.buf.len() > MAX_RESPONSE {
                return Err(Error::protocol("response too large"));
            }
            let mut chunk = [0u8; 16384];
            let n = self.stream.read(&mut chunk).await?;
            if n == 0 {
                return Err(Error::protocol("connection closed"));
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Sends a request and waits for the response with the same id. Event lines are skipped.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        tokio::time::timeout(TIMEOUT, self.call_inner(method, params))
            .await
            .map_err(|_| Error::Timeout("api call"))?
    }

    async fn call_inner(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut line = json!({"id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        self.stream.write_all(line.as_bytes()).await?;
        self.stream.flush().await?;
        loop {
            let raw = self.read_line().await?;
            let v: Value = serde_json::from_slice(&raw)?;
            if v.get("id").and_then(Value::as_str) != Some(&id) {
                continue;
            }
            if let Some(e) = v.get("error") {
                return Err(Error::Api {
                    code: e.get("code").map(|c| c.to_string().trim_matches('"').to_string()).unwrap_or_default(),
                    message: e.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
                });
            }
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    pub async fn snapshot(&mut self) -> Result<Snapshot> {
        let r = self.call("session.snapshot", json!({})).await?;
        let snap = r.get("snapshot").ok_or_else(|| Error::protocol("no snapshot in result"))?;
        Ok(Snapshot { workspaces: nest_snapshot(snap)?, focused_tab_id: os(snap, "focused_tab_id") })
    }

    /// Subscribes to change events; afterwards `next_event` yields event lines.
    pub async fn subscribe(&mut self) -> Result<()> {
        let types = [
            "workspace.created", "workspace.updated", "workspace.metadata_updated", "workspace.renamed",
            "workspace.moved", "workspace.reordered", "workspace.closed", "tab.created", "tab.closed",
            "tab.renamed", "tab.moved", "pane.created", "pane.closed", "pane.updated", "pane.moved",
            "pane.exited", "pane.agent_detected", "layout.updated",
        ];
        let subs: Vec<Value> = types.iter().map(|t| json!({"type": t})).collect();
        let r = self.call("events.subscribe", json!({"subscriptions": subs})).await?;
        if r.get("type").and_then(Value::as_str) != Some("subscription_started") {
            return Err(Error::protocol("subscription not started"));
        }
        Ok(())
    }

    /// Waits for the next event line (no timeout; caller wraps it).
    pub async fn next_event(&mut self) -> Result<Value> {
        loop {
            let raw = self.read_line().await?;
            let v: Value = serde_json::from_slice(&raw)?;
            if v.get("event").is_some() {
                return Ok(v);
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub workspaces: Vec<Workspace>,
    pub focused_tab_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Pane {
    pub id: String,
    pub focused: bool,
    pub cwd: Option<String>,
    pub agent_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Tab {
    pub id: String,
    pub label: String,
    pub focused: bool,
    pub panes: Vec<Pane>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Workspace {
    pub id: String,
    pub label: String,
    pub focused: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_state: Option<String>,
    /// Labels of other clients attached to this Space, best effort from the snapshot; absent = unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence: Option<Vec<String>>,
    pub tabs: Vec<Tab>,
}

/// Best-effort attach list: `presence` / `clients` / `attached_clients` on the workspace, as strings or
/// objects with a `label`/`name`/`client_name`. None when the snapshot carries none of these.
fn presence_of(w: &Value) -> Option<Vec<String>> {
    let a = ["presence", "attached_clients", "clients"].iter().find_map(|k| w.get(*k).and_then(Value::as_array))?;
    Some(
        a.iter()
            .filter_map(|c| match c {
                Value::String(x) => Some(x.clone()),
                _ => ["label", "name", "client_name"].iter().find_map(|k| os(c, k)),
            })
            .collect(),
    )
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn os(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(String::from)
}
fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(|a| a.as_slice()).unwrap_or(&[])
}

/// Nests the flat snapshot into workspaces -> tabs -> panes.
pub fn nest_snapshot(snap: &Value) -> Result<Vec<Workspace>> {
    let (tabs, panes) = (arr(snap, "tabs"), arr(snap, "panes"));
    Ok(arr(snap, "workspaces")
        .iter()
        .map(|w| {
            let wid = s(w, "workspace_id");
            Workspace {
                tabs: tabs
                    .iter()
                    .filter(|t| s(t, "workspace_id") == wid)
                    .map(|t| {
                        let tid = s(t, "tab_id");
                        Tab {
                            label: s(t, "label"),
                            focused: t.get("focused").and_then(Value::as_bool).unwrap_or(false),
                            panes: panes
                                .iter()
                                .filter(|p| s(p, "tab_id") == tid)
                                .map(|p| Pane {
                                    id: s(p, "pane_id"),
                                    focused: p.get("focused").and_then(Value::as_bool).unwrap_or(false),
                                    cwd: os(p, "cwd"),
                                    agent_status: os(p, "agent_status"),
                                })
                                .collect(),
                            id: tid,
                        }
                    })
                    .collect(),
                label: s(w, "label"),
                focused: w.get("focused").and_then(Value::as_bool).unwrap_or(false),
                agent_state: os(w, "agent_status"),
                presence: presence_of(w),
                id: wid,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nests_flat_snapshot() {
        let snap = json!({
            "workspaces": [{"workspace_id":"w1","label":"A","focused":true,"agent_status":"idle"}],
            "tabs": [{"tab_id":"t1","workspace_id":"w1","label":"main","focused":true},
                     {"tab_id":"t2","workspace_id":"w2","label":"other"}],
            "panes": [{"pane_id":"p1","tab_id":"t1","focused":true,"cwd":"/x"},
                      {"pane_id":"p2","tab_id":"t2"}]
        });
        let w = nest_snapshot(&snap).unwrap();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].tabs.len(), 1);
        assert_eq!(w[0].tabs[0].panes[0].id, "p1");
        assert_eq!(w[0].agent_state.as_deref(), Some("idle"));
    }

    #[test]
    fn presence_is_best_effort() {
        let snap = json!({"workspaces": [
            {"workspace_id":"w1","clients":[{"label":"studio-mac"},"win-pc"]},
            {"workspace_id":"w2"}]});
        let w = nest_snapshot(&snap).unwrap();
        assert_eq!(w[0].presence.as_deref(), Some(&["studio-mac".to_string(), "win-pc".to_string()][..]));
        assert_eq!(w[1].presence, None);
    }

    #[test]
    fn real_fixture_snapshot_nests() {
        let p = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/snapshot.json");
        let v: Value = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        let snap = v.get("result").and_then(|r| r.get("snapshot")).unwrap_or(&v);
        let snap = snap.get("snapshot").unwrap_or(snap);
        let w = nest_snapshot(snap).unwrap();
        assert!(!w.is_empty());
        assert!(!w[0].tabs.is_empty());
    }
}
