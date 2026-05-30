//! Optional turn timing sink for gateway solve (`.claw/solve-timing-events.ndjson`). Author: kejiqing

use serde_json::{Map, Value};

/// Append-only timing events (implemented in `gateway-solve-turn`).
pub trait TurnTimingSink: Send + Sync {
    fn emit(&self, kind: &str, attributes: Map<String, Value>);
}
