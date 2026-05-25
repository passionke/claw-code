//! MCP `tools/call` `_meta`: business context in `extra_session` + claw correlation keys. Author: kejiqing

use serde_json::{json, Map, Value};

use crate::GatewaySolveTaskFile;

/// Injected into `_meta.extra_session` (underscore prefix avoids clashing with business keys).
pub const CLAW_EXTRA_SESSION_SESSION_ID: &str = "_claw_session_id";
pub const CLAW_EXTRA_SESSION_TURN_ID: &str = "_claw_turn_id";

/// Correlation ids for one gateway solve turn (injected into MCP `_meta.extra_session` only).
#[derive(Debug, Clone)]
pub struct GatewayMcpCallContext {
    pub session_id: String,
    pub turn_id: String,
    /// HTTP/async solve job id (trace file key when `CLAW_TRACE_ID` unset).
    pub request_id: String,
    pub trace_id: String,
    /// Normalized gateway `extraSession` without `_claw_*` keys.
    pub extra_session: Option<Value>,
}

impl GatewayMcpCallContext {
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
    pub fn from_task(task: &GatewaySolveTaskFile) -> Self {
        let session_id = task
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(task.request_id.as_str());
        let turn_id = {
            let t = task.turn_id.trim();
            if t.is_empty() {
                task.request_id.as_str()
            } else {
                t
            }
        };
        Self::new(
            session_id,
            turn_id,
            task.request_id.as_str(),
            task.extra_session.clone(),
        )
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
        None => Map::new(),
        Some(Value::Object(map)) => map,
        Some(_) => Map::new(),
    }
}

/// Merge gateway correlation into `extra_session` and wrap as MCP `_meta`. Author: kejiqing
#[must_use]
pub fn build_mcp_call_meta(ctx: &GatewayMcpCallContext) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize_extra_session;
    use serde_json::json;

    #[test]
    fn from_task_prefers_session_id_over_request_id() {
        let task = GatewaySolveTaskFile {
            request_id: "req-1".into(),
            user_prompt: "q".into(),
            model: None,
            timeout_seconds: None,
            extra_session: None,
            allowed_tools: None,
            max_iterations: None,
            turn_id: "T_abc".into(),
            session_id: Some("sess-stable".into()),
            pool_id: None,
            worker_name: None,
        };
        let ctx = GatewayMcpCallContext::from_task(&task);
        assert_eq!(ctx.session_id, "sess-stable");
        assert_eq!(ctx.request_id, "req-1");
        assert_eq!(ctx.turn_id, "T_abc");
    }

    #[test]
    fn meta_is_only_extra_session_with_claw_keys() {
        let extra = normalize_extra_session(Some(json!({"store_id": "S1"}))).unwrap();
        let ctx = GatewayMcpCallContext::new("sess", "T_1", "req-9", Some(extra));
        let meta = build_mcp_call_meta(&ctx);
        assert_eq!(meta.as_object().map(|m| m.len()), Some(1));
        let es = &meta["extra_session"];
        assert_eq!(es[CLAW_EXTRA_SESSION_SESSION_ID], "sess");
        assert_eq!(es[CLAW_EXTRA_SESSION_TURN_ID], "T_1");
        assert_eq!(es["store_id"], "S1");
        assert!(meta.get("claw").is_none());
        assert!(meta.get("session_id").is_none());
    }
}
