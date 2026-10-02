//! `.neuro-harness/turn-context.json`: per-turn MCP correlation written by the solve process and
//! read by `mcp-proxy` on every `tools/call`. Author: kejiqing

use std::path::{Path, PathBuf};

use gateway_solve_turn::GatewaySolveTaskFile;
use runtime::McpCallContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{HarnessError, HARNESS_DIR};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnContext {
    pub session_id: String,
    pub turn_id: String,
    pub request_id: String,
    pub trace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_session: Option<Value>,
}

impl TurnContext {
    /// Same resolution as claw (`gateway_mcp_call_context_from_task`), trace id fixed once per turn.
    #[must_use]
    pub fn from_task(task: &GatewaySolveTaskFile) -> Self {
        let ctx = gateway_solve_turn::gateway_mcp_call_context_from_task(task);
        Self {
            session_id: ctx.session_id,
            turn_id: ctx.turn_id,
            request_id: ctx.request_id,
            trace_id: ctx.trace_id,
            extra_session: ctx.extra_session,
        }
    }

    #[must_use]
    pub fn to_mcp_call_context(&self) -> McpCallContext {
        McpCallContext {
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            request_id: self.request_id.clone(),
            trace_id: self.trace_id.clone(),
            extra_session: self.extra_session.clone(),
        }
    }
}

#[must_use]
pub fn turn_context_path(session_root: &Path) -> PathBuf {
    session_root.join(HARNESS_DIR).join("turn-context.json")
}

pub fn write_turn_context(session_root: &Path, ctx: &TurnContext) -> Result<(), HarnessError> {
    let path = turn_context_path(session_root);
    let body = serde_json::to_vec_pretty(ctx)
        .map_err(|e| HarnessError::internal(format!("serialize turn context: {e}")))?;
    std::fs::write(&path, body)
        .map_err(|e| HarnessError::internal(format!("write {}: {e}", path.display())))
}

pub fn read_turn_context(session_root: &Path) -> Result<TurnContext, String> {
    let path = turn_context_path(session_root);
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_keeps_trace_id_and_builds_claw_meta() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(HARNESS_DIR)).unwrap();
        let task: GatewaySolveTaskFile = serde_json::from_value(json!({
            "requestId":"r1","userPrompt":"hi","turnId":"t1","sessionId":"s1",
            "extraSession":{"org_id":"o1","trace_id":"tr-1"}
        }))
        .unwrap();
        let ctx = TurnContext::from_task(&task);
        write_turn_context(dir.path(), &ctx).unwrap();
        let back = read_turn_context(dir.path()).unwrap();
        assert_eq!(back, ctx);
        let meta = runtime::build_mcp_call_meta(&back.to_mcp_call_context());
        assert_eq!(meta["extra_session"]["org_id"], "o1");
        assert_eq!(meta["extra_session"]["trace_id"], "tr-1");
        assert_eq!(meta["extra_session"]["_claw_session_id"], "s1");
        assert_eq!(meta["extra_session"]["_claw_turn_id"], "t1");
    }
}
