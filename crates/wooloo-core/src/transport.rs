//! Byte-stream transports to a Herdr server: Unix socket, Windows named pipe, system ssh.
use crate::machines::{valid_session, Machine, MachineKind};
use crate::{Error, Result};
use async_trait::async_trait;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub trait AsyncStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncStream for T {}
pub type BoxStream = Box<dyn AsyncStream>;

/// Which of the two Herdr endpoints to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// Newline-delimited JSON socket API (`herdr.sock`, `remote-api-bridge`).
    Api,
    /// Binary generation-1 client endpoint (`herdr-client.sock`, `remote-client-bridge`).
    Client,
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn connect(&self, channel: Channel) -> Result<BoxStream>;
    /// Last diagnostic text (e.g. ssh stderr tail), if any.
    fn diagnostics(&self) -> Option<String> {
        None
    }
}

/// Builds the right transport for a machine.
pub fn for_machine(m: &Machine) -> Result<Arc<dyn Transport>> {
    m.validate()?;
    Ok(match m.kind {
        MachineKind::Local => {
            #[cfg(unix)]
            {
                Arc::new(UnixTransport { session: m.session.clone() })
            }
            #[cfg(windows)]
            {
                Arc::new(PipeTransport { session: m.session.clone(), pipe_path: m.pipe_path.clone() })
            }
        }
        MachineKind::Ssh => Arc::new(SshTransport::new(
            m.ssh_target.clone().unwrap_or_default(),
            m.session.clone(),
        )),
    })
}

// ---------------------------------------------------------------- Unix socket

#[cfg(unix)]
pub struct UnixTransport {
    pub session: Option<String>,
}

#[cfg(unix)]
impl UnixTransport {
    pub fn path(&self, channel: Channel) -> std::path::PathBuf {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
        let base = home.join(".config").join("herdr");
        let dir = match &self.session {
            Some(s) => base.join("sessions").join(s),
            None => base,
        };
        dir.join(match channel {
            Channel::Api => "herdr.sock",
            Channel::Client => "herdr-client.sock",
        })
    }
}

#[cfg(unix)]
#[async_trait]
impl Transport for UnixTransport {
    async fn connect(&self, channel: Channel) -> Result<BoxStream> {
        if let Some(s) = &self.session {
            if !valid_session(s) {
                return Err(Error::Other("invalid session name".into()));
            }
        }
        let s = tokio::time::timeout(Duration::from_secs(5), tokio::net::UnixStream::connect(self.path(channel)))
            .await
            .map_err(|_| Error::Timeout("connect"))??;
        Ok(Box::new(s))
    }
}

// ---------------------------------------------------------------- Windows named pipe

/// Herdr (src/platform/windows.rs) binds the pipe `\\.\pipe\` + the *socket file path* string, i.e.
/// `\\.\pipe\C:\Users\me\AppData\Roaming\herdr\sessions\<s>\herdr.sock` (API) and `...\herdr-client.sock`
/// (binary). The directory is `config_dir()` (src/config/io.rs): `$XDG_CONFIG_HOME\herdr`, else
/// `%APPDATA%\herdr` (release builds; debug builds use `herdr-dev`). Read from the herdr source, not the
/// docs; not yet exercised against a real Windows server (UNVERIFIED on hardware).
/// `pipe_path` overrides the API socket path (client path derives `herdr.sock` -> `herdr-client.sock`).
pub fn pipe_name(session: &Option<String>, channel: Channel, pipe_path: &Option<String>) -> String {
    let api_path = match pipe_path {
        Some(p) if !p.is_empty() => p.clone(),
        _ => {
            let base = std::env::var("XDG_CONFIG_HOME")
                .map(|d| format!(r"{d}\herdr"))
                .or_else(|_| std::env::var("APPDATA").map(|d| format!(r"{d}\herdr")))
                .or_else(|_| std::env::var("USERPROFILE").map(|d| format!(r"{d}\AppData\Roaming\herdr")))
                .unwrap_or_else(|_| r"C:\herdr".into());
            match session.as_deref() {
                Some(s) => format!(r"{base}\sessions\{s}\herdr.sock"),
                None => format!(r"{base}\herdr.sock"),
            }
        }
    };
    let path = match channel {
        Channel::Api => api_path,
        Channel::Client => match api_path.strip_suffix(".sock") {
            Some(stem) => format!("{stem}-client.sock"),
            None => format!("{api_path}-client.sock"),
        },
    };
    format!(r"\\.\pipe\{path}")
}

#[cfg(windows)]
pub struct PipeTransport {
    pub session: Option<String>,
    pub pipe_path: Option<String>,
}

#[cfg(windows)]
#[async_trait]
impl Transport for PipeTransport {
    async fn connect(&self, channel: Channel) -> Result<BoxStream> {
        if let Some(s) = &self.session {
            if !valid_session(s) {
                return Err(Error::Other("invalid session name".into()));
            }
        }
        let name = pipe_name(&self.session, channel, &self.pipe_path);
        let c = tokio::net::windows::named_pipe::ClientOptions::new().open(&name)?;
        Ok(Box::new(c))
    }
}

// ---------------------------------------------------------------- SSH

pub struct SshTransport {
    target: String,
    session: Option<String>,
    stderr_tail: Arc<Mutex<String>>,
}

/// POSIX single-quote a string.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The remote shell script that locates a compatible `herdr` and execs the bridge.
pub fn remote_script(session: &str, bridge: &str) -> String {
    let q = sh_quote(session);
    format!(
        r#"for h in "$(command -v herdr 2>/dev/null)" "$HOME/.local/bin/herdr" /opt/homebrew/bin/herdr /usr/local/bin/herdr /home/linuxbrew/.linuxbrew/bin/herdr "$HOME/.cargo/bin/herdr" "$HOME/.local/share/mise/shims/herdr" "$HOME/.nix-profile/bin/herdr" /run/current-system/sw/bin/herdr /nix/var/nix/profiles/default/bin/herdr; do
  [ -n "$h" ] && [ -x "$h" ] || continue
  if [ "$("$h" --session {q} remote-api-bridge --check 2>/dev/null)" = herdr-api-bridge-v1 ]; then
    exec "$h" --session {q} {bridge}
  fi
done
echo "A compatible Herdr is not installed on this host" >&2
exit 127"#
    )
}

impl SshTransport {
    pub fn new(target: String, session: Option<String>) -> Self {
        SshTransport { target, session, stderr_tail: Arc::new(Mutex::new(String::new())) }
    }

    /// ssh argv (without the program name).
    pub fn args(&self, channel: Channel) -> Result<Vec<String>> {
        let (host, port) = split_target(&self.target);
        if host.is_empty() || host.starts_with('-') {
            return Err(Error::Other("invalid ssh target".into()));
        }
        let session = self.session.clone().unwrap_or_else(|| "default".into());
        if !session.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') || session.len() > 64 {
            return Err(Error::Other("session name not allowed over ssh bridge".into()));
        }
        let bridge = match channel {
            Channel::Api => "remote-api-bridge",
            Channel::Client => "remote-client-bridge",
        };
        let mut a: Vec<String> = ["-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "-o", "ServerAliveInterval=10", "-o", "ServerAliveCountMax=2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        #[cfg(unix)]
        {
            // Windows OpenSSH has no connection reuse.
            let dir = std::env::temp_dir().join("wooloo-ssh");
            let _ = std::fs::create_dir_all(&dir);
            a.extend([
                "-o".into(),
                "ControlMaster=auto".into(),
                "-o".into(),
                format!("ControlPath={}/%C", dir.display()),
                "-o".into(),
                "ControlPersist=60".into(),
            ]);
        }
        if let Some(p) = port {
            a.push("-p".into());
            a.push(p.to_string());
        }
        a.push(host.to_string());
        a.push(format!("/bin/sh -c {}", sh_quote(&remote_script(&session, bridge))));
        Ok(a)
    }
}

/// Splits `user@host:port` (a bare `host:port` too); IPv6 literals are left alone.
pub fn split_target(t: &str) -> (&str, Option<u16>) {
    if let Some((h, p)) = t.rsplit_once(':') {
        if !h.contains(':') {
            if let Ok(p) = p.parse::<u16>() {
                return (h, Some(p));
            }
        }
    }
    (t, None)
}

pub struct SshStream {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
}

impl AsyncRead for SshStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdout).poll_read(cx, buf)
    }
}
impl AsyncWrite for SshStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stdin).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdin).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stdin).poll_shutdown(cx)
    }
}
impl Drop for SshStream {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[async_trait]
impl Transport for SshTransport {
    async fn connect(&self, channel: Channel) -> Result<BoxStream> {
        let program = if cfg!(windows) { "ssh.exe" } else { "ssh" };
        let mut cmd = tokio::process::Command::new(program);
        cmd.args(self.args(channel)?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| Error::Other("no ssh stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| Error::Other("no ssh stdout".into()))?;
        // stderr never enters the protocol stream; keep only a short tail for diagnostics.
        if let Some(mut err) = child.stderr.take() {
            let tail = self.stderr_tail.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut buf = [0u8; 1024];
                while let Ok(n) = err.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let mut t = tail.lock().unwrap();
                    t.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if t.len() > 4000 {
                        let mut cut = t.len() - 2000;
                        while !t.is_char_boundary(cut) {
                            cut += 1;
                        }
                        *t = t[cut..].to_string();
                    }
                }
            });
        }
        Ok(Box::new(SshStream { child, stdin, stdout }))
    }

    fn diagnostics(&self) -> Option<String> {
        let t = self.stderr_tail.lock().unwrap().trim().to_string();
        if t.is_empty() { None } else { Some(t) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn target_split() {
        assert_eq!(split_target("me@box:2222"), ("me@box", Some(2222)));
        assert_eq!(split_target("me@box"), ("me@box", None));
    }

    #[test]
    fn ssh_args_reject_dash() {
        assert!(SshTransport::new("-oFoo".into(), None).args(Channel::Api).is_err());
        let a = SshTransport::new("me@box:22".into(), Some("wooloo".into())).args(Channel::Api).unwrap();
        assert!(a.contains(&"-p".to_string()) && a.contains(&"me@box".to_string()));
        assert!(a.last().unwrap().contains("remote-api-bridge"));
    }
}
