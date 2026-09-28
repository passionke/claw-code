//! MCP `tools/call` `_meta`: business context in `extra_session` + claw correlation keys. Author: kejiqing

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

/// Injected into `_meta.extra_session` (underscore prefix avoids clashing with business keys).
pub const CLAW_EXTRA_SESSION_SESSION_ID: &str = "_claw_session_id";
pub const CLAW_EXTRA_SESSION_TURN_ID: &str = "_claw_turn_id";

/// Distributed log correlation key in `extra_session` / MCP `_meta.extra_session`. Author: kejiqing
pub const EXTRA_SESSION_TRACE_ID: &str = "trace_id";

static TRACE_FALLBACK_SEQ: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static CURRENT_MCP_CALL_CONTEXT: RefCell<Option<McpCallContext>> = const { RefCell::new(None) };
}

/// Correlation ids for one solve turn (injected into MCP `_meta.extra_session`).
#[derive(Debug, Clone)]
pub struct McpCallContext {
    pub session_id: String,
    pub turn_id: String,
    /// HTTP/async solve job id (often equals sessionId; not used as distributed trace).
    pub request_id: String,
    /// Distributed log correlation id (independent of sessionId). Author: kejiqing
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
        // Prefer extra_session.trace_id (HTTP injects). Never default to request_id/sessionId.
        let mut trace_id = resolve_gateway_trace_id(extra_session.as_ref(), "");
        if trace_id.is_empty() {
            trace_id = mint_gateway_trace_id_fallback();
        }
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

    /// MCP `tools/call` `_meta`: `{ "extra_session": { …business, "trace_id", "_claw_*" } }`.
    #[must_use]
    pub fn to_mcp_meta(&self) -> Value {
        build_mcp_call_meta(self)
    }
}

/// Non-empty trimmed string, or `None`. Author: kejiqing
#[must_use]
pub fn non_empty_trace_id(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Read `extra_session.trace_id` when present and non-empty. Author: kejiqing
#[must_use]
pub fn trace_id_from_extra_session(extra_session: Option<&Value>) -> Option<String> {
    let Value::Object(map) = extra_session? else {
        return None;
    };
    let Value::String(v) = map.get(EXTRA_SESSION_TRACE_ID)? else {
        return None;
    };
    non_empty_trace_id(v)
}

/// Resolve distributed `trace_id` (not sessionId).
///
/// Priority: `extra_session.trace_id` → `generated_fallback` → `CLAW_TRACE_ID` env.
/// Author: kejiqing
#[must_use]
pub fn resolve_gateway_trace_id(extra_session: Option<&Value>, generated_fallback: &str) -> String {
    if let Some(tid) = trace_id_from_extra_session(extra_session) {
        return tid;
    }
    if let Some(tid) = non_empty_trace_id(generated_fallback) {
        return tid;
    }
    std::env::var("CLAW_TRACE_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_default()
}

/// Process-local unique fallback when HTTP did not inject (32 hex). Author: kejiqing
#[must_use]
pub fn mint_gateway_trace_id_fallback() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = TRACE_FALLBACK_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{nanos:016x}{seq:016x}")
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
        EXTRA_SESSION_TRACE_ID.to_string(),
        Value::String(ctx.trace_id.clone()),
    );
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
    use std::sync::Mutex;

    /// Serialize env-touching tests. Author: kejiqing
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: Mutex<()> = Mutex::new(());
        &LOCK
    }

    #[test]
    fn meta_includes_trace_id_and_claw_keys() {
        let ctx = McpCallContext::new(
            "sess",
            "T_1",
            "req-9",
            Some(json!({
                "store_id": "S1",
                "org_id": "",
                "trace_id": "aabbccddeeff00112233445566778899"
            })),
        );
        let meta = build_mcp_call_meta(&ctx);
        assert_eq!(meta.as_object().map(serde_json::Map::len), Some(1));
        let es = &meta["extra_session"];
        assert_eq!(es[CLAW_EXTRA_SESSION_SESSION_ID], "sess");
        assert_eq!(es[CLAW_EXTRA_SESSION_TURN_ID], "T_1");
        assert_eq!(
            es[EXTRA_SESSION_TRACE_ID],
            "aabbccddeeff00112233445566778899"
        );
        assert_eq!(es["store_id"], "S1");
        assert_eq!(ctx.trace_id, "aabbccddeeff00112233445566778899");
        assert!(meta.get("claw").is_none());
        assert!(meta.get("session_id").is_none());
    }

    #[test]
    fn meta_injects_trace_id_when_caller_omitted() {
        let ctx = McpCallContext::new("sess", "T_1", "sess", Some(json!({"store_id": "S1"})));
        let meta = build_mcp_call_meta(&ctx);
        let tid = meta["extra_session"][EXTRA_SESSION_TRACE_ID]
            .as_str()
            .expect("trace_id in meta");
        assert!(!tid.is_empty());
        assert_ne!(tid, "sess");
        assert_eq!(ctx.trace_id, tid);
    }

    #[test]
    fn resolve_prefers_extra_session_over_fallback_and_env() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::set_var("CLAW_TRACE_ID", "from-env");
        let extra = json!({"trace_id": "from-body"});
        assert_eq!(
            resolve_gateway_trace_id(Some(&extra), "from-fallback"),
            "from-body"
        );
        std::env::remove_var("CLAW_TRACE_ID");
    }

    #[test]
    fn resolve_prefers_fallback_over_env() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::set_var("CLAW_TRACE_ID", "from-env");
        assert_eq!(
            resolve_gateway_trace_id(None, "from-fallback"),
            "from-fallback"
        );
        std::env::remove_var("CLAW_TRACE_ID");
    }

    #[test]
    fn resolve_uses_env_when_no_extra_or_fallback() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::set_var("CLAW_TRACE_ID", "from-env");
        assert_eq!(resolve_gateway_trace_id(None, ""), "from-env");
        std::env::remove_var("CLAW_TRACE_ID");
    }

    #[test]
    fn resolve_does_not_use_request_id_as_trace() {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::env::remove_var("CLAW_TRACE_ID");
        // Empty → empty string (caller mints). Never silently use session/request id.
        assert_eq!(resolve_gateway_trace_id(None, ""), "");
        let ctx = McpCallContext::new("session-abc", "T_1", "session-abc", None);
        assert_ne!(ctx.trace_id, "session-abc");
        assert!(!ctx.trace_id.is_empty());
    }

    #[test]
    fn inject_and_build_meta_agree_on_trace_id() {
        let ctx = McpCallContext::new(
            "sess",
            "T_1",
            "req",
            Some(json!({"trace_id": "11112222333344445555666677778888"})),
        );
        assert_eq!(
            build_mcp_call_meta(&ctx)["extra_session"][EXTRA_SESSION_TRACE_ID],
            inject_mcp_call_meta(&ctx)["extra_session"][EXTRA_SESSION_TRACE_ID]
        );
        assert_eq!(
            ctx.to_mcp_meta()["extra_session"][EXTRA_SESSION_TRACE_ID],
            "11112222333344445555666677778888"
        );
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
