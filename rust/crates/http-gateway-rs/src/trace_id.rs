//! Request-level distributed `trace_id` (`X-Trace-Id` / `extra_session.trace_id`). Author: kejiqing
//!
//! Distinct from `sessionId` (conversation + business records). Used for log correlation across
//! KEY, Gateway worker, and MCP `_meta.extra_session.trace_id`.

use runtime::{non_empty_trace_id, resolve_gateway_trace_id, EXTRA_SESSION_TRACE_ID};
use serde_json::{json, Map, Value};
use uuid::Uuid;

/// HTTP request/response header for distributed log correlation. Author: kejiqing
pub const HEADER_TRACE_ID: &str = "x-trace-id";

/// Mint a new gateway `trace_id` (UUID v4 without dashes → 32 hex). Author: kejiqing
#[must_use]
pub fn mint_request_trace_id() -> String {
    Uuid::new_v4().simple().to_string()
}

/// Resolve inbound trace id: header → `extra_session.trace_id` → mint new (≠ sessionId).
/// Author: kejiqing
#[must_use]
pub fn resolve_request_trace_id(header: Option<&str>, extra_session: Option<&Value>) -> String {
    if let Some(h) = header.and_then(non_empty_trace_id) {
        return h;
    }
    let generated = mint_request_trace_id();
    resolve_gateway_trace_id(extra_session, &generated)
}

/// Ensure `extra_session.trace_id` holds the effective value (HTTP → task → worker). Author: kejiqing
pub fn ensure_extra_session_trace_id(extra_session: &mut Option<Value>, trace_id: &str) {
    let tid = trace_id.trim();
    if tid.is_empty() {
        return;
    }
    match extra_session {
        Some(Value::Object(map)) => {
            map.insert(
                EXTRA_SESSION_TRACE_ID.to_string(),
                Value::String(tid.to_string()),
            );
        }
        Some(_) | None => {
            let mut map = Map::new();
            map.insert(
                EXTRA_SESSION_TRACE_ID.to_string(),
                Value::String(tid.to_string()),
            );
            *extra_session = Some(Value::Object(map));
        }
    }
}

/// Resolve from headers + body, write back into `extra_session`, return effective id. Author: kejiqing
#[must_use]
pub fn apply_inbound_trace_id(header: Option<&str>, extra_session: &mut Option<Value>) -> String {
    let tid = resolve_request_trace_id(header, extra_session.as_ref());
    ensure_extra_session_trace_id(extra_session, &tid);
    tid
}

/// Read `X-Trace-Id` from a header map. Author: kejiqing
#[must_use]
pub fn trace_id_from_headers(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(HEADER_TRACE_ID)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use runtime::{
        build_mcp_call_meta, inject_mcp_call_meta, McpCallContext, EXTRA_SESSION_TRACE_ID,
    };

    #[test]
    fn header_overrides_body() {
        let extra = json!({"trace_id": "from-body", "store_id": "S1"});
        let tid = resolve_request_trace_id(Some("from-header"), Some(&extra));
        assert_eq!(tid, "from-header");
    }

    #[test]
    fn body_used_when_no_header() {
        let extra = json!({"trace_id": "from-body"});
        let tid = resolve_request_trace_id(None, Some(&extra));
        assert_eq!(tid, "from-body");
    }

    #[test]
    fn mints_when_absent() {
        let tid = resolve_request_trace_id(None, None);
        assert_eq!(tid.len(), 32);
        assert!(tid.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn ensure_writes_trace_id() {
        let mut extra = Some(json!({"store_id": "S1"}));
        ensure_extra_session_trace_id(&mut extra, "abc123");
        assert_eq!(extra.as_ref().unwrap()["trace_id"], "abc123");
        assert_eq!(extra.as_ref().unwrap()["store_id"], "S1");
    }

    #[test]
    fn apply_inbound_injects_minted() {
        let mut extra = None;
        let tid = apply_inbound_trace_id(None, &mut extra);
        assert_eq!(extra.as_ref().unwrap()["trace_id"], tid);
    }

    #[test]
    fn header_map_read() {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER_TRACE_ID, HeaderValue::from_static("hdr-tid"));
        assert_eq!(trace_id_from_headers(&headers), Some("hdr-tid"));
    }

    /// HTTP header → extra_session → McpCallContext → MCP `_meta` 同值，且 ≠ sessionId. Author: kejiqing
    #[test]
    fn pipeline_header_survives_to_mcp_meta() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HEADER_TRACE_ID,
            HeaderValue::from_static("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        );
        let mut extra = Some(json!({
            "store_id": "S1",
            "trace_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        }));
        let tid = apply_inbound_trace_id(trace_id_from_headers(&headers), &mut extra);
        assert_eq!(tid, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(extra.as_ref().unwrap()[EXTRA_SESSION_TRACE_ID], tid);

        let session = "session-should-not-be-trace";
        let ctx = McpCallContext::new(session, "T_turn1", session, extra.clone());
        assert_eq!(ctx.trace_id, tid);
        assert_ne!(ctx.trace_id, ctx.session_id);
        assert_ne!(ctx.trace_id, ctx.request_id);

        let meta = build_mcp_call_meta(&ctx);
        assert_eq!(meta["extra_session"][EXTRA_SESSION_TRACE_ID], tid);
        assert_eq!(
            inject_mcp_call_meta(&ctx)["extra_session"][EXTRA_SESSION_TRACE_ID],
            tid
        );
    }

    /// KEY 只传 body.trace_id 时贯通到 meta. Author: kejiqing
    #[test]
    fn pipeline_body_survives_to_mcp_meta() {
        let mut extra = Some(json!({
            "org_id": "",
            "trace_id": "cccccccccccccccccccccccccccccccc"
        }));
        let tid = apply_inbound_trace_id(None, &mut extra);
        assert_eq!(tid, "cccccccccccccccccccccccccccccccc");

        let ctx = McpCallContext::new("sess-1", "T_1", "sess-1", extra);
        assert_eq!(ctx.trace_id, tid);
        assert_eq!(
            build_mcp_call_meta(&ctx)["extra_session"][EXTRA_SESSION_TRACE_ID],
            tid
        );
    }

    /// 未传时网关铸造，写入 extra_session 且进 meta，不等于 sessionId. Author: kejiqing
    #[test]
    fn pipeline_minted_trace_not_equal_session_reaches_meta() {
        let session = "dddddddddddddddddddddddddddddddd";
        let mut extra = Some(json!({"store_id": "S9"}));
        let tid = apply_inbound_trace_id(None, &mut extra);
        assert_eq!(tid.len(), 32);
        assert_ne!(tid, session);
        assert_eq!(extra.as_ref().unwrap()[EXTRA_SESSION_TRACE_ID], tid);

        let ctx = McpCallContext::new(session, "T_1", session, extra);
        assert_eq!(ctx.trace_id, tid);
        assert_ne!(ctx.trace_id, session);
        assert_eq!(
            build_mcp_call_meta(&ctx)["extra_session"][EXTRA_SESSION_TRACE_ID],
            tid
        );
    }

    /// 空白头应回落到 body，避免「假头」打断贯通. Author: kejiqing
    #[test]
    fn blank_header_falls_through_to_body() {
        let mut extra = Some(json!({"trace_id": "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"}));
        let tid = apply_inbound_trace_id(Some("   "), &mut extra);
        assert_eq!(tid, "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
    }
}
