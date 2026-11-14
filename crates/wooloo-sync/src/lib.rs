//! wooloo-sync: replicated Space metadata between the user's machines.
//!
//! Implements `docs/sync-protocol.md` v1: a last-writer-wins register map with
//! hybrid logical clocks, an allow-list validator, Space binding, the unit-state
//! categories bridge, the state file and the line-delimited exchange protocol.

pub mod canonical;
pub mod doc;
pub mod exchange;
pub mod hlc;
pub mod state;
pub mod unitcat;
pub mod validate;

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

pub use doc::{Absorbed, BindKind, BindPlan, Binding, Outcome, Register, Rejection, Replica};
pub use hlc::{now_ms, Hlc};

/// Ops whose wall clock is further ahead of the receiver than this are deferred.
pub const MAX_FUTURE_MS: u64 = 600_000;
/// Exchange protocol version (section 8).
pub const PROTO: u64 = 1;
/// State file format (section 6).
pub const FORMAT: u64 = 1;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Invalid(String),
    Protocol(String),
    Remote(String),
    Busy(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "i/o error: {e}"),
            Error::Invalid(m) => write!(f, "invalid data: {m}"),
            Error::Protocol(m) => write!(f, "protocol error: {m}"),
            Error::Remote(m) => write!(f, "peer reported an error: {m}"),
            Error::Busy(m) => write!(f, "busy: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// `path` with `suffix` appended to its file name (`state.json` -> `state.json.tmp`).
pub(crate) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}
