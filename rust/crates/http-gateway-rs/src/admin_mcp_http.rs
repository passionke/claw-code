//! Streamable-HTTP MCP server for gateway admin (`POST /v1/admin/mcp`). Author: kejiqing

use axum::body::Bytes;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::gateway_admin_mcp_token::{extract_bearer_token, verify_admin_mcp_token};
use crate::project_config_draft;
use crate::session_db::{GatewaySessionDb, ProjectConfigUpsert};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_SERVER_NAME: &str = "claw-gateway-admin";
const MCP_SERVER_VERSION: &str = "0.1.0";

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse<'a> {
    jsonrpc: &'a str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcErrorObj>,
}

#[derive(Debug, Serialize)]
struct JsonRpcErrorObj {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

pub async fn handle_admin_mcp_post(
    db: &GatewaySessionDb,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let Some(token) = extract_bearer_token(bearer) else {
        return mcp_auth_error_response("missing Authorization: Bearer <admin-mcp-token>");
    };
    if let Err(msg) = verify_admin_mcp_token(db, &token).await {
        return mcp_auth_error_response(&msg);
    }

    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return json_rpc_error_response(
                Value::Null,
                -32700,
                format!("parse error: {e}"),
                StatusCode::BAD_REQUEST,
            );
        }
    };
    let id = request.id.clone().unwrap_or(Value::Null);
    let session_id = headers
        .get("Mcp-Session-Id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let result: Result<Value, String> = match request.method.as_str() {
        "initialize" => Ok(handle_initialize(request.params.as_ref())),
        "notifications/initialized" | "initialized" | "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list_result()),
        "tools/call" => match handle_tools_call(db, request.params).await {
            Ok(v) => Ok(v),
            Err(e) => {
                return json_rpc_error_response(id, -32000, e, StatusCode::OK);
            }
        },
        other => {
            return json_rpc_error_response(
                id,
                -32601,
                format!("method not found: {other}"),
                StatusCode::OK,
            );
        }
    };

    match result {
        Ok(value) => json_rpc_ok_response(id, value, &session_id),
        Err(e) => json_rpc_error_response(id, -32603, e, StatusCode::OK),
    }
}

fn handle_initialize(params: Option<&Value>) -> Value {
    let _ = params;
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": MCP_SERVER_NAME,
            "version": MCP_SERVER_VERSION
        }
    })
}

fn tools_list_result() -> Value {
    json!({
        "tools": [
            {
                "name": "project_config_get",
                "description": "Read project_config row for projId (draft when open, else active).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "projId": { "type": "integer", "description": "Project id" }
                    },
                    "required": ["projId"]
                }
            },
            {
                "name": "project_claude_get",
                "description": "Read CLAUDE.md content from project_config.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "projId": { "type": "integer" }
                    },
                    "required": ["projId"]
                }
            },
            {
                "name": "project_claude_put_draft",
                "description": "Write claudeMd into open draft (__draft__); does not activate.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "projId": { "type": "integer" },
                        "content": { "type": "string" }
                    },
                    "required": ["projId", "content"]
                }
            }
        ]
    })
}

async fn handle_tools_call(db: &GatewaySessionDb, params: Option<Value>) -> Result<Value, String> {
    let params = params.ok_or_else(|| "params required".to_string())?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "params.name required".to_string())?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let proj_id = args
        .get("projId")
        .and_then(Value::as_i64)
        .ok_or_else(|| "arguments.projId required".to_string())?;

    match name {
        "project_config_get" => {
            let row = db
                .get_project_config(proj_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("project_config not found for projId={proj_id}"))?;
            let payload = json!({
                "projId": row.proj_id,
                "contentRev": row.content_rev,
                "stableContentRev": row.stable_content_rev,
                "draftOpen": row.draft_open,
                "claudeMd": row.claude_md,
                "rulesJson": row.rules_json,
                "skillsJson": row.skills_json,
                "mcpServersJson": row.mcp_servers_json,
                "allowedToolsJson": row.allowed_tools_json,
            });
            Ok(tool_text_result(&payload))
        }
        "project_claude_get" => {
            let row = db
                .get_project_config(proj_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("project_config not found for projId={proj_id}"))?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "content": row.claude_md.unwrap_or_default()
            })))
        }
        "project_claude_put_draft" => {
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| "arguments.content required".to_string())?;
            project_config_draft::ensure_draft(db, proj_id)
                .await
                .map_err(|e| e.message)?;
            let existing = db
                .get_project_config(proj_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("project_config not found for projId={proj_id}"))?;
            let effective = project_config_draft::effective_formal_rev(&existing)
                .map_err(|e| e.message)?
                .to_string();
            let now = now_ms();
            let claude_md = if content.trim().is_empty() {
                None
            } else {
                Some(content.to_string())
            };
            let upsert = ProjectConfigUpsert {
                proj_id,
                content_rev: project_config_draft::DRAFT_CONTENT_REV,
                stable_content_rev: Some(effective.as_str()),
                draft_open: true,
                updated_at_ms: now,
                rules_json: &existing.rules_json,
                mcp_servers_json: &existing.mcp_servers_json,
                skills_sources_json: &existing.skills_sources_json,
                skills_json: &existing.skills_json,
                allowed_tools_json: &existing.allowed_tools_json,
                claude_md: claude_md.as_deref(),
                git_sync_json: &existing.git_sync_json,
                solve_preflight_json: &existing.solve_preflight_json,
                solve_orchestration_json: &existing.solve_orchestration_json,
                language_pipeline_json: &existing.language_pipeline_json,
                extra_session_fields_json: &existing.extra_session_fields_json,
                prompt_limits_json: &existing.prompt_limits_json,
                worker_isolation_json: &existing.worker_isolation_json,
            };
            db.upsert_project_config(upsert)
                .await
                .map_err(|e| e.to_string())?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "updated": true,
                "bytes": content.len()
            })))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

fn tool_text_result(value: &Value) -> Value {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn json_rpc_ok_response(id: Value, result: Value, session_id: &str) -> Response {
    let body = JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    if let Ok(v) = HeaderValue::from_str(session_id) {
        headers.insert("Mcp-Session-Id", v);
    }
    if let Ok(v) = HeaderValue::from_str(MCP_PROTOCOL_VERSION) {
        headers.insert("MCP-Protocol-Version", v);
    }
    (StatusCode::OK, headers, Json(body)).into_response()
}

fn json_rpc_error_response(
    id: Value,
    code: i32,
    message: impl Into<String>,
    status: StatusCode,
) -> Response {
    let body = JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcErrorObj {
            code,
            message: message.into(),
            data: None,
        }),
    };
    (status, Json(body)).into_response()
}

fn mcp_auth_error_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json")],
        Json(json!({ "error": message })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_has_project_config_get() {
        let v = tools_list_result();
        let names: Vec<_> = v["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        assert!(names.contains(&"project_config_get"));
    }
}
