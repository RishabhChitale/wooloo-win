//! wooloo-core: Herdr client core (no Tauri dependency).
pub mod api;
pub mod discover;
pub mod error;
pub mod machines;
pub mod manager;
pub mod meta;
pub mod pairing;
pub mod surface;
pub mod transport;

pub use error::{Error, Result};
