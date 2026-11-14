//! `machines.toml` model and load/save.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MachineKind {
    Local,
    Ssh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    pub id: String,
    pub label: String,
    pub kind: MachineKind,
    /// `user@host` or `user@host:port` (ssh only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_target: Option<String>,
    /// Herdr session name; `None` = the default session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Local Windows only: full path of the API socket file whose name Herdr turns into a pipe
    /// (`\\.\pipe\<path>`), e.g. `C:\Users\me\AppData\Roaming\herdr\herdr.sock`. Overrides the derived path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipe_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachinesFile {
    #[serde(default, rename = "machine")]
    pub machines: Vec<Machine>,
}

/// Valid Herdr session name: `^[A-Za-z0-9_.-]+$`, <= 64 bytes, not "." or "..".
pub fn valid_session(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s != "."
        && s != ".."
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

impl Machine {
    pub fn validate(&self) -> Result<()> {
        if let Some(s) = &self.session {
            if !valid_session(s) {
                return Err(Error::Other(format!("invalid session name: {s}")));
            }
        }
        if self.kind == MachineKind::Ssh {
            match self.ssh_target.as_deref() {
                Some(t) if !t.is_empty() && !t.starts_with('-') => {}
                _ => return Err(Error::Other("ssh machine needs a valid ssh_target".into())),
            }
        }
        Ok(())
    }
}

impl MachinesFile {
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| Error::Toml(e.to_string()))
    }

    pub fn to_string(&self) -> Result<String> {
        toml::to_string_pretty(self).map_err(|e| Error::Toml(e.to_string()))
    }

    /// Loads the file; a missing file yields a file holding just the local machine.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(t) => Self::parse(&t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::with_local()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_string()?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn with_local() -> Self {
        MachinesFile {
            machines: vec![Machine {
                id: "local".into(),
                label: "This computer".into(),
                kind: MachineKind::Local,
                ssh_target: None,
                session: None,
                pipe_path: None,
            }],
        }
    }

    pub fn add_ssh(&mut self, label: String, ssh_target: String, session: Option<String>) -> Result<Machine> {
        let m = Machine {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            label,
            kind: MachineKind::Ssh,
            ssh_target: Some(ssh_target),
            session: session.filter(|s| !s.is_empty()),
            pipe_path: None,
        };
        m.validate()?;
        self.machines.push(m.clone());
        Ok(m)
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let n = self.machines.len();
        self.machines.retain(|m| m.id != id);
        self.machines.len() != n
    }

    pub fn get(&self, id: &str) -> Option<&Machine> {
        self.machines.iter().find(|m| m.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut f = MachinesFile::with_local();
        let m = f.add_ssh("Box".into(), "me@box:2222".into(), Some("wooloo".into())).unwrap();
        let text = f.to_string().unwrap();
        let g = MachinesFile::parse(&text).unwrap();
        assert_eq!(f, g);
        assert_eq!(g.get(&m.id).unwrap().session.as_deref(), Some("wooloo"));
    }

    #[test]
    fn rejects_bad_input() {
        let mut f = MachinesFile::default();
        assert!(f.add_ssh("x".into(), "-oProxyCommand=x".into(), None).is_err());
        assert!(f.add_ssh("x".into(), "a@b".into(), Some("../x".into())).is_err());
        assert!(valid_session("wooloo-win_test.1"));
        assert!(!valid_session(".."));
    }
}
