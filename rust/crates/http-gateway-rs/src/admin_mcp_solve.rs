//! Admin MCP solve / async solve bridge. Author: kejiqing

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::project_extra_session;
use crate::session_db::GatewaySessionDb;

/// Solve request parsed from admin MCP tool arguments. Author: kejiqing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminMcpSolveInput {
    #[serde(rename = "projId")]
    pub proj_id: i64,
    #[serde(rename = "userPrompt")]
    pub user_prompt: String,
    #[serde(default, rename = "sessionId")]
    pub session_id: Option<String>,
    pub model: Option<String>,
    #[serde(rename = "timeoutSeconds")]
    pub timeout_seconds: Option<u64>,
    #[serde(rename = "extraSession")]
    pub extra_session: Option<Value>,
    #[serde(rename = "allowedTools")]
    pub allowed_tools: Option<Vec<String>>,
}

/// Backend implemented by the gateway binary (`AppState`). Author: kejiqing
#[async_trait]
pub trait AdminMcpSolveBackend: Send + Sync {
    async fn gateway_solve_sync(&self, input: AdminMcpSolveInput) -> Result<Value, String>;
    async fn gateway_solve_async(&self, input: AdminMcpSolveInput) -> Result<Value, String>;
    async fn gateway_task_get(&self, task_id: &str) -> Result<Value, String>;
}

/// Parse and validate solve tool arguments (project extraSession fields). Author: kejiqing
pub async fn validate_admin_mcp_solve_input(
    db: &GatewaySessionDb,
    input: &AdminMcpSolveInput,
) -> Result<(), String> {
    if input.proj_id < 1 {
        return Err("projId must be >= 1".to_string());
    }
    if input.user_prompt.trim().is_empty() {
        return Err("userPrompt cannot be empty".to_string());
    }
    validate_extra_session_payload(input.extra_session.as_ref())?;
    let row = db
        .get_project_config(input.proj_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project_config not found for projId={}", input.proj_id))?;
    let fields =
        project_extra_session::parse_extra_session_fields_json(&row.extra_session_fields_json)?;
    project_extra_session::validate_extra_session_against_fields(
        input.extra_session.as_ref(),
        &fields,
    )
    .map_err(|e| e.clone())
}

pub fn parse_solve_tool_args(args: &Value) -> Result<AdminMcpSolveInput, String> {
    serde_json::from_value(args.clone()).map_err(|e| format!("invalid solve tool arguments: {e}"))
}

fn validate_extra_session_payload(extra_session: Option<&Value>) -> Result<(), String> {
    if let Some(extra_session) = extra_session {
        if !extra_session.is_object() {
            return Err("extraSession must be a JSON object when present".to_string());
        }
        if let Ok(serialized) = serde_json::to_vec(extra_session) {
            if serialized.len() > 8 * 1024 {
                return Err("extraSession is too large (max 8KB)".to_string());
            }
        }
    }
    Ok(())
}

pub fn solve_tools_schema() -> Vec<Value> {
    let solve_props = json!({
        "projId": {
            "type": "integer",
            "description": "Project id (>= 1). Call project_list or project_extra_session_fields_get first."
        },
        "userPrompt": {
            "type": "string",
            "description": "User natural-language question for this solve turn."
        },
        "sessionId": {
            "type": "string",
            "description": "Optional. Pass the sessionId from a prior solve response to continue the same conversation."
        },
        "extraSession": {
            "type": "object",
            "description": "Business context object. Must include all string fields required by project_extra_session_fields_get for this projId.",
            "additionalProperties": { "type": "string" }
        },
        "model": { "type": "string", "description": "Optional model override." },
        "timeoutSeconds": { "type": "integer", "description": "Optional overall timeout in seconds." },
        "allowedTools": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Optional per-request tool allowlist."
        }
    });
    let solve_required = &["projId", "userPrompt"];
    vec![
        json!({
            "name": "project_list",
            "description": "List known projId values (DB project_config + disk proj_* dirs) with required extraSession field names per project.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "project_extra_session_fields_get",
            "description": "Return extraSessionFieldsJson (required string keys) for a projId before calling gateway_solve or gateway_solve_async.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projId": { "type": "integer", "description": "Project id" }
                },
                "required": ["projId"]
            }
        }),
        json!({
            "name": "gateway_solve",
            "description": "Synchronously run one gateway solve (resolve) turn. Returns full SolveResponse including sessionId and turnId for continuation.",
            "inputSchema": {
                "type": "object",
                "properties": solve_props,
                "required": solve_required
            }
        }),
        json!({
            "name": "gateway_solve_async",
            "description": "Enqueue async solve; returns taskId/sessionId/turnId. Poll gateway_task_get until status is terminal.",
            "inputSchema": {
                "type": "object",
                "properties": solve_props,
                "required": solve_required
            }
        }),
        json!({
            "name": "gateway_task_get",
            "description": "Poll async solve task status/result by taskId (same value as sessionId from gateway_solve_async).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "taskId": {
                        "type": "string",
                        "description": "taskId from gateway_solve_async (equals sessionId)"
                    }
                },
                "required": ["taskId"]
            }
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solve_tools_include_gateway_and_project_list() {
        let tools = solve_tools_schema();
        let names: Vec<_> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        for expected in [
            "project_list",
            "project_extra_session_fields_get",
            "gateway_solve",
            "gateway_solve_async",
            "gateway_task_get",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn parse_solve_tool_args_accepts_session_id() {
        let args = json!({
            "projId": 10,
            "userPrompt": "hello",
            "sessionId": "abc",
            "extraSession": { "store_id": "S1" }
        });
        let input = parse_solve_tool_args(&args).unwrap();
        assert_eq!(input.proj_id, 10);
        assert_eq!(input.session_id.as_deref(), Some("abc"));
    }
}
