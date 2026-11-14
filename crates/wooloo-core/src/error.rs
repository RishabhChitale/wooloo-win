use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml: {0}")]
    Toml(String),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("api error {code}: {message}")]
    Api { code: String, message: String },
    #[error("timeout: {0}")]
    Timeout(&'static str),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn protocol(s: impl Into<String>) -> Self {
        Error::Protocol(s.into())
    }
}
