//! MCP `tools/call` `_meta`: business context in `extra_session` + claw correlation keys. Author: kejiqing

use std::cell::RefCell;

use serde_json::{json, Map, Value};

/// Injected into `_meta.extra_session` (underscore prefix avoids clashing with business keys).
pub const CLAW_EXTRA_SESSION_SESSION_ID: &str = "_claw_session_id";
pub const CLAW_EXTRA_SESSION_TURN_ID: &str = "_claw_turn_id";

thread_local! {
    static CURRENT_MCP_CALL_CONTEXT: RefCell<Option<McpCallContext>> = const { RefCell::new(None) };
}

/// Correlation ids for one solve turn (injected into MCP `_meta.extra_session` only).
#[derive(Debug, Clone)]
pub struct McpCallContext {
    pub session_id: String,
    pub turn_id: String,
    /// HTTP/async solve job id (trace file key when `CLAW_TRACE_ID` unset).
    pub request_id: String,
    pub trace_id: String,
    /// Normalized gateway `extraSession` without `_claw_*` keys.
    pub extra_session: Option<Value>,
}

impl McpCallContext {
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        request_id: impl Into<String>,
        extra_session: Option<Value>,
    ) -> Self {
        let request_id = request_id.into();
        let trace_id = resolve_gateway_trace_id(&request_id);
        Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            request_id,
            trace_id,
            extra_session,
        }
    }

    #[must_use]
    pub fn clawcode_session_id(&self) -> &str {
        self.session_id.as_str()
    }

    /// MCP `tools/call` `_meta`: `{ "extra_session": { …business, "_claw_session_id", "_claw_turn_id" } }`.
    #[must_use]
    pub fn to_mcp_meta(&self) -> Value {
        build_mcp_call_meta(self)
    }
}

#[must_use]
pub fn resolve_gateway_trace_id(request_id: &str) -> String {
    std::env::var("CLAW_TRACE_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| request_id.to_string())
}

fn extra_session_object(extra_session: Option<Value>) -> Map<String, Value> {
    match extra_session {
        Some(Value::Object(map)) => map,
        Some(_) | None => Map::new(),
    }
}

/// Merge correlation into `extra_session` and wrap as MCP `_meta`. Author: kejiqing
#[must_use]
pub fn build_mcp_call_meta(ctx: &McpCallContext) -> Value {
    let mut extra = extra_session_object(ctx.extra_session.clone());
    extra.insert(
        CLAW_EXTRA_SESSION_SESSION_ID.to_string(),
        Value::String(ctx.session_id.clone()),
    );
    extra.insert(
        CLAW_EXTRA_SESSION_TURN_ID.to_string(),
        Value::String(ctx.turn_id.clone()),
    );
    json!({ "extra_session": Value::Object(extra) })
}

/// Single injection point for MCP `tools/call` `_meta`. Author: kejiqing
#[must_use]
pub fn inject_mcp_call_meta(ctx: &McpCallContext) -> Value {
    ctx.to_mcp_meta()
}

/// Same-thread scoped MCP context (e.g. nested tool dispatch). Subagent threads must pass context explicitly. Author: kejiqing
pub fn with_mcp_call_context<R>(ctx: McpCallContext, f: impl FnOnce() -> R) -> R {
    CURRENT_MCP_CALL_CONTEXT.with(|slot| {
        let prev = slot.replace(Some(ctx));
        let out = f();
        slot.replace(prev);
        out
    })
}

#[must_use]
pub fn current_mcp_call_context() -> Option<McpCallContext> {
    CURRENT_MCP_CALL_CONTEXT.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn meta_is_only_extra_session_with_claw_keys() {
        let ctx = McpCallContext::new(
            "sess",
            "T_1",
            "req-9",
            Some(json!({"store_id": "S1", "org_id": ""})),
        );
        let meta = build_mcp_call_meta(&ctx);
        assert_eq!(meta.as_object().map(|m| m.len()), Some(1));
        let es = &meta["extra_session"];
        assert_eq!(es[CLAW_EXTRA_SESSION_SESSION_ID], "sess");
        assert_eq!(es[CLAW_EXTRA_SESSION_TURN_ID], "T_1");
        assert_eq!(es["store_id"], "S1");
        assert!(meta.get("claw").is_none());
        assert!(meta.get("session_id").is_none());
    }

    #[test]
    fn with_mcp_call_context_scopes_current() {
        assert!(current_mcp_call_context().is_none());
        let ctx = McpCallContext::new("s", "T", "r", None);
        with_mcp_call_context(ctx.clone(), || {
            assert_eq!(
                current_mcp_call_context()
                    .as_ref()
                    .map(|c| c.session_id.as_str()),
                Some("s")
            );
        });
        assert!(current_mcp_call_context().is_none());
    }
}
