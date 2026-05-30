//! Optional turn timing sink for gateway solve (`.claw/solve-timing-events.ndjson`). Author: kejiqing

use serde_json::{Map, Value};
use std::cell::Cell;

thread_local! {
    static TOOL_TIMING_FROM_LOOP: Cell<bool> = const { Cell::new(false) };
}

/// True while `ConversationRuntime` is executing tools inside the main loop (avoid double-counting).
#[must_use]
pub fn conversation_tool_timing_from_loop() -> bool {
    TOOL_TIMING_FROM_LOOP.with(std::cell::Cell::get)
}

pub(crate) fn set_conversation_tool_timing_from_loop(active: bool) {
    TOOL_TIMING_FROM_LOOP.with(|c| c.set(active));
}

/// Append-only timing events (implemented in `gateway-solve-turn`).
pub trait TurnTimingSink: Send + Sync {
    fn emit(&self, kind: &str, attributes: Map<String, Value>);
}
