//! Hybrid logical clock (section 2.2).

use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// `(wall, counter)`; the derived order is lexicographic, as the spec requires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    pub wall: u64,
    pub counter: u64,
}

impl Hlc {
    pub const ZERO: Hlc = Hlc { wall: 0, counter: 0 };

    pub fn new(wall: u64, counter: u64) -> Hlc {
        Hlc { wall, counter }
    }

    /// Local write at physical time `now` (ms). Returns the write's hlc.
    pub fn tick(&mut self, now: u64) -> Hlc {
        if now > self.wall {
            *self = Hlc { wall: now, counter: 0 };
        } else {
            // Saturating so a hostile peer's u64::MAX counter cannot panic us.
            self.counter = self.counter.saturating_add(1);
        }
        *self
    }

    /// Accepted remote op: take its clock if it is ahead of ours.
    pub fn observe(&mut self, remote: Hlc) {
        if remote > *self {
            *self = remote;
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "wall": self.wall, "counter": self.counter })
    }

    pub fn from_json(v: &Value) -> Option<Hlc> {
        let o = v.as_object()?;
        Some(Hlc {
            wall: o.get("wall")?.as_u64()?,
            counter: o.get("counter")?.as_u64()?,
        })
    }
}

/// Milliseconds since the Unix epoch (0 if the system clock is before it).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
