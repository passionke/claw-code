//! OTEL spans for tool execution (independent of JSONL `SessionTracer`). Author: kejiqing

use std::cell::RefCell;
use std::collections::HashMap;

use telemetry::{otel_enabled, OtelSpanGuard};

thread_local! {
    static TOOL_OTEL_GUARDS: RefCell<HashMap<String, OtelSpanGuard>> =
        RefCell::new(HashMap::new());
}

pub fn otel_tool_started(tool_use_id: &str, tool_name: &str) {
    if !otel_enabled() {
        return;
    }
    let Some(guard) = OtelSpanGuard::start("claw-runtime", "tool.execution", None) else {
        return;
    };
    guard.set_attribute(
        "langfuse.observation.metadata.tool_name",
        tool_name.to_string(),
    );
    guard.set_attribute("tool.name", tool_name.to_string());
    guard.set_attribute("tool.use_id", tool_use_id.to_string());
    TOOL_OTEL_GUARDS.with(|map| {
        map.borrow_mut().insert(tool_use_id.to_string(), guard);
    });
}

pub fn otel_tool_finished(tool_use_id: &str, is_error: bool, duration_ms: u128) {
    if !otel_enabled() {
        return;
    }
    TOOL_OTEL_GUARDS.with(|map| {
        let Some(guard) = map.borrow_mut().remove(tool_use_id) else {
            return;
        };
        guard.set_attribute("duration_ms", duration_ms.to_string());
        guard.set_attribute("tool.is_error", is_error.to_string());
        if is_error {
            guard.set_error("tool_execution_failed");
        } else {
            guard.set_ok();
        }
    });
}
