//! Gateway solve MCP call context: resolve entry + re-exports from `runtime`. Author: kejiqing

use runtime::McpCallContext;
use serde_json::{json, Map, Value};

use crate::{normalize_extra_session, GatewaySolveTaskFile};

pub use runtime::{
    build_mcp_call_meta, inject_mcp_call_meta, resolve_gateway_trace_id, with_mcp_call_context,
    CLAW_EXTRA_SESSION_SESSION_ID, CLAW_EXTRA_SESSION_TURN_ID,
};

/// Stable alias for gateway / CLI callers. Author: kejiqing
pub type GatewayMcpCallContext = McpCallContext;

/// Resolve normalized MCP call context from gateway solve inputs (single entry point). Author: kejiqing
#[must_use]
pub fn resolve_gateway_mcp_call_context(
    session_id: impl Into<String>,
    turn_id: impl Into<String>,
    request_id: impl Into<String>,
    extra_session: Option<Value>,
) -> McpCallContext {
    McpCallContext::new(
        session_id,
        turn_id,
        request_id,
        normalize_extra_session(extra_session),
    )
}

/// Build MCP call context from a gateway solve task file. Author: kejiqing
#[must_use]
pub fn gateway_mcp_call_context_from_task(task: &GatewaySolveTaskFile) -> McpCallContext {
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
    resolve_gateway_mcp_call_context(
        session_id,
        turn_id,
        task.request_id.as_str(),
        task.extra_session.clone(),
    )
}

/// SQLBot `mcp_start` **arguments**: business fields from normalized `extraSession`.
///
/// Strips `_claw_*` keys (those belong in `_meta` via `inject_mcp_call_meta` only).
/// SQLBot binds store/org on start; later MCP tools keep `token` only in arguments.
/// Author: kejiqing
#[must_use]
pub fn build_sqlbot_mcp_start_arguments(extra_session: Option<Value>) -> Value {
    let Some(Value::Object(map)) = extra_session else {
        return json!({});
    };
    let mut out = Map::new();
    for (key, value) in map {
        if key.starts_with("_claw_") {
            continue;
        }
        out.insert(key, value);
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            llm_route: None,
            otel_traceparent: None,
        };
        let ctx = gateway_mcp_call_context_from_task(&task);
        assert_eq!(ctx.session_id, "sess-stable");
        assert_eq!(ctx.request_id, "req-1");
        assert_eq!(ctx.turn_id, "T_abc");
    }

    #[test]
    fn resolve_matches_from_task_extra_session() {
        let task = GatewaySolveTaskFile {
            request_id: "req-1".into(),
            user_prompt: "q".into(),
            model: None,
            timeout_seconds: None,
            extra_session: Some(json!({"store_id": "S1"})),
            allowed_tools: None,
            max_iterations: None,
            turn_id: "T_1".into(),
            session_id: Some("sess".into()),
            pool_id: None,
            worker_name: None,
            llm_route: None,
            otel_traceparent: None,
        };
        let from_task = gateway_mcp_call_context_from_task(&task);
        let resolved = resolve_gateway_mcp_call_context(
            "sess",
            "T_1",
            "req-1",
            Some(json!({"store_id": "S1"})),
        );
        assert_eq!(from_task.session_id, resolved.session_id);
        assert_eq!(from_task.turn_id, resolved.turn_id);
        let meta = build_mcp_call_meta(&resolved);
        assert_eq!(meta["extra_session"]["store_id"], "S1");
        assert_eq!(meta["extra_session"][CLAW_EXTRA_SESSION_TURN_ID], "T_1");
    }

    #[test]
    fn mcp_start_arguments_copy_business_keys_only() {
        let extra = normalize_extra_session(Some(json!({
            "store_id": "S1",
            "tenant_code": "GPOS",
            "org_id": ""
        })));
        let args = build_sqlbot_mcp_start_arguments(extra);
        assert_eq!(args["store_id"], "S1");
        assert_eq!(args["tenant_code"], "GPOS");
        assert_eq!(args["org_id"], "");
        assert!(args.get("_claw_session_id").is_none());
    }

    #[test]
    fn mcp_start_arguments_empty_when_no_extra_session() {
        assert_eq!(build_sqlbot_mcp_start_arguments(None), json!({}));
    }

    #[test]
    fn mcp_start_arguments_from_normalize_none_has_org_id() {
        let args = build_sqlbot_mcp_start_arguments(normalize_extra_session(None));
        assert_eq!(args, json!({"org_id": ""}));
    }
}
