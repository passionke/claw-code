//! `complete_router_turn`: explicit router harness terminal after successful delegate(s).
//! Author: kejiqing
//!
//! Returns [`runtime::ToolLoopDirective::CompleteTurn`] so ConversationRuntime ends the
//! turn without another LLM call. Validates that this router turn already has a
//! non-empty `.claw/delegate-agents-results/<routerTurnId>.md` report file.

use std::fs;
use std::path::PathBuf;

use api::ToolDefinition;
use runtime::ToolError;
use serde_json::json;

use crate::delegate_project_tool::router_delegate_report_rel;
use crate::mcp_call_context::GatewayMcpCallContext;

pub const COMPLETE_ROUTER_TURN_TOOL_NAME: &str = "complete_router_turn";

#[must_use]
pub fn complete_router_turn_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: COMPLETE_ROUTER_TURN_TOOL_NAME.to_string(),
        description: Some(
            "End this router turn after successful delegate_project_tool call(s). \
             Specialist body already streamed to the user; do not emit text. \
             Call only when no further delegate is needed."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

fn session_home_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Validate delegated report artifact and return tool-result JSON. Author: kejiqing
pub fn run_complete_router_turn(mcp: &GatewayMcpCallContext) -> Result<String, ToolError> {
    let router_turn = mcp.turn_id.trim();
    if router_turn.is_empty() {
        return Err(ToolError::new(
            "complete_router_turn: router turnId required",
        ));
    }
    let rel = router_delegate_report_rel(router_turn);
    let path = session_home_dir().join(&rel);
    let meta = fs::metadata(&path).map_err(|_| {
        ToolError::new(format!(
            "complete_router_turn: no successful delegate report at {rel}"
        ))
    })?;
    if meta.len() == 0 {
        return Err(ToolError::new(format!(
            "complete_router_turn: delegate report empty at {rel}"
        )));
    }
    serde_json::to_string_pretty(&json!({
        "status": "completed",
        "routerSessionId": mcp.clawcode_session_id(),
        "routerTurnId": router_turn,
        "reportPath": rel,
        "reportBytes": meta.len(),
    }))
    .map_err(|e| ToolError::new(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime::McpCallContext;
    use serde_json::Value;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// `run_complete_router_turn` resolves report paths via `current_dir`; serialize cwd tests.
    /// Author: kejiqing
    static CWD_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn rejects_missing_report_file() {
        let _guard = CWD_LOCK.lock().expect("cwd lock");
        let dir = tempdir().expect("temp");
        let prev = std::env::current_dir().ok();
        std::env::set_current_dir(dir.path()).expect("cd");
        let mcp = McpCallContext::new("sess", "T_missing", "req", None);
        let err = run_complete_router_turn(&mcp).expect_err("missing");
        assert!(err.to_string().contains("no successful delegate"));
        if let Some(p) = prev {
            let _ = std::env::set_current_dir(p);
        }
    }

    #[test]
    fn rejects_empty_report_file() {
        let _guard = CWD_LOCK.lock().expect("cwd lock");
        let dir = tempdir().expect("temp");
        let prev = std::env::current_dir().ok();
        std::env::set_current_dir(dir.path()).expect("cd");
        let rel = router_delegate_report_rel("T_empty");
        let path = dir.path().join(&rel);
        fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        fs::write(&path, "").expect("write empty");
        let mcp = McpCallContext::new("sess", "T_empty", "req", None);
        let err = run_complete_router_turn(&mcp).expect_err("empty");
        assert!(err.to_string().contains("empty"), "error={}", err);
        if let Some(p) = prev {
            let _ = std::env::set_current_dir(p);
        }
    }

    #[test]
    fn rejects_blank_turn_id() {
        let mcp = McpCallContext::new("sess", "   ", "req", None);
        let err = run_complete_router_turn(&mcp).expect_err("blank turn");
        assert!(err.to_string().contains("turnId"));
    }

    #[test]
    fn accepts_non_empty_report_file() {
        let _guard = CWD_LOCK.lock().expect("cwd lock");
        let dir = tempdir().expect("temp");
        let prev = std::env::current_dir().ok();
        std::env::set_current_dir(dir.path()).expect("cd");
        let rel = router_delegate_report_rel("T_ok");
        let path = dir.path().join(&rel);
        fs::create_dir_all(path.parent().unwrap()).expect("mkdir");
        fs::write(&path, "specialist body").expect("write");
        assert!(path.is_file(), "report must exist at {}", path.display());
        let mcp = McpCallContext::new("sess", "T_ok", "req", None);
        let out = run_complete_router_turn(&mcp).expect("ok");
        let v: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("completed"));
        assert_eq!(
            v.get("reportPath").and_then(|s| s.as_str()),
            Some(rel.as_str())
        );
        assert_eq!(
            v.get("reportBytes").and_then(|b| b.as_u64()),
            Some("specialist body".len() as u64)
        );
        assert!(v.get("message").is_none());
        if let Some(p) = prev {
            let _ = std::env::set_current_dir(p);
        }
    }

    #[test]
    fn tool_definition_exposes_stable_name() {
        let def = complete_router_turn_tool_definition();
        assert_eq!(def.name, COMPLETE_ROUTER_TURN_TOOL_NAME);
        assert!(def
            .description
            .as_deref()
            .unwrap_or("")
            .contains("End this router turn"));
    }
}
