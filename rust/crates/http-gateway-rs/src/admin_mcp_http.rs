//! Streamable-HTTP MCP server for gateway admin (`POST /v1/admin/mcp`). Author: kejiqing

use axum::body::Bytes;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::gateway_admin_mcp_token::{extract_bearer_token, verify_admin_mcp_token};
use crate::gateway_global_settings;
use crate::pool::NasLayoutBackend;
use crate::project_config_apply;
use crate::project_config_draft;
use crate::session_db::{GatewaySessionDb, ProjectConfigRow, ProjectConfigUpsert};
use std::path::{Path, PathBuf};

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

/// CLAUDE.md field update for draft patch.
enum ClaudeMdPatch<'a> {
    Unchanged,
    Clear,
    Set(&'a str),
}

/// Fields to replace on the open `__draft__` row; omitted fields are copied from current draft/base.
struct DraftPatch<'a> {
    claude_md: ClaudeMdPatch<'a>,
    rules_json: Option<&'a Value>,
    mcp_servers_json: Option<&'a Value>,
    skills_json: Option<&'a Value>,
}

impl Default for ClaudeMdPatch<'_> {
    fn default() -> Self {
        Self::Unchanged
    }
}

impl Default for DraftPatch<'_> {
    fn default() -> Self {
        Self {
            claude_md: ClaudeMdPatch::Unchanged,
            rules_json: None,
            mcp_servers_json: None,
            skills_json: None,
        }
    }
}

pub async fn handle_admin_mcp_post(
    db: &GatewaySessionDb,
    work_root: &Path,
    nas_layout: &NasLayoutBackend,
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
        "tools/call" => match handle_tools_call(db, work_root, nas_layout, request.params).await {
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

fn tool_def(name: &str, description: &str, properties: &Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required
        }
    })
}

fn proj_id_only_tool(name: &str, description: &str) -> Value {
    tool_def(
        name,
        description,
        &json!({ "projId": { "type": "integer", "description": "Project id" } }),
        &["projId"],
    )
}

fn tools_list_result() -> Value {
    json!({
        "tools": [
            proj_id_only_tool(
                "project_config_get",
                "Read full project_config row for projId (draft when open, else effective formal).",
            ),
            tool_def(
                "project_config_put_draft",
                "Write claudeMd / rulesJson / mcpServersJson / skillsJson into open draft; pass at least one field.",
                &json!({
                    "projId": { "type": "integer" },
                    "claudeMd": { "type": "string" },
                    "rulesJson": { "type": "array" },
                    "mcpServersJson": { "type": "object" },
                    "skillsJson": { "type": "array" }
                }),
                &["projId"],
            ),
            tool_def(
                "project_config_commit_draft",
                "Save open draft as a new formal version (does not activate).",
                &json!({
                    "projId": { "type": "integer" },
                    "note": { "type": "string" }
                }),
                &["projId"],
            ),
            tool_def(
                "project_config_activate",
                "Activate a saved formal contentRev and materialize to project work dir.",
                &json!({
                    "projId": { "type": "integer" },
                    "contentRev": { "type": "string" }
                }),
                &["projId", "contentRev"],
            ),
            proj_id_only_tool(
                "project_claude_get",
                "Read CLAUDE.md content from project_config.",
            ),
            tool_def(
                "project_claude_put_draft",
                "Write claudeMd into open draft (__draft__); does not activate.",
                &json!({
                    "projId": { "type": "integer" },
                    "content": { "type": "string" }
                }),
                &["projId", "content"],
            ),
            proj_id_only_tool(
                "project_rules_get",
                "Read rulesJson array from project_config.",
            ),
            tool_def(
                "project_rules_put_draft",
                "Write rulesJson into open draft (__draft__); does not activate. Items: relativePath, content, optional enabled.",
                &json!({
                    "projId": { "type": "integer" },
                    "rulesJson": { "type": "array" }
                }),
                &["projId", "rulesJson"],
            ),
            proj_id_only_tool(
                "project_mcp_get",
                "Read mcpServersJson object from project_config (solve MCP servers, not this admin MCP endpoint).",
            ),
            tool_def(
                "project_mcp_put_draft",
                "Write mcpServersJson into open draft (__draft__); does not activate.",
                &json!({
                    "projId": { "type": "integer" },
                    "mcpServersJson": { "type": "object" }
                }),
                &["projId", "mcpServersJson"],
            ),
            proj_id_only_tool(
                "project_skills_get",
                "Read skillsJson array from project_config.",
            ),
            tool_def(
                "project_skills_put_draft",
                "Write skillsJson into open draft (__draft__); does not activate. Items: skillName, skillContent, optional enabled.",
                &json!({
                    "projId": { "type": "integer" },
                    "skillsJson": { "type": "array" }
                }),
                &["projId", "skillsJson"],
            ),
        ]
    })
}

async fn row_for_editing_or_err(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<ProjectConfigRow, String> {
    project_config_draft::row_for_editing(db, proj_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project_config not found for projId={proj_id}"))
}

async fn upsert_project_draft(
    db: &GatewaySessionDb,
    proj_id: i64,
    patch: DraftPatch<'_>,
) -> Result<(), String> {
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
    let claude_md = match patch.claude_md {
        ClaudeMdPatch::Unchanged => existing.claude_md.as_deref(),
        ClaudeMdPatch::Clear => None,
        ClaudeMdPatch::Set(s) => Some(s),
    };
    let rules_json = patch.rules_json.unwrap_or(&existing.rules_json);
    let mcp_servers_json = patch.mcp_servers_json.unwrap_or(&existing.mcp_servers_json);
    let skills_json = patch.skills_json.unwrap_or(&existing.skills_json);
    let now = now_ms();
    let upsert = ProjectConfigUpsert {
        proj_id,
        content_rev: project_config_draft::DRAFT_CONTENT_REV,
        stable_content_rev: Some(effective.as_str()),
        draft_open: true,
        updated_at_ms: now,
        rules_json,
        mcp_servers_json,
        skills_sources_json: &existing.skills_sources_json,
        skills_json,
        allowed_tools_json: &existing.allowed_tools_json,
        claude_md,
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
        .map_err(|e| e.to_string())
}

fn validate_rules_json(v: &Value) -> Result<(), String> {
    if !v.is_array() {
        return Err("rulesJson must be a JSON array".to_string());
    }
    Ok(())
}

fn validate_mcp_servers_json(v: &Value) -> Result<(), String> {
    if !v.is_object() {
        return Err("mcpServersJson must be a JSON object".to_string());
    }
    Ok(())
}

fn validate_skill_name(skill_name: &str) -> Result<(), String> {
    if skill_name.trim().is_empty() {
        return Err("skillName cannot be empty".to_string());
    }
    if skill_name
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'))
    {
        return Err("skillName only allows [a-zA-Z0-9._-]".to_string());
    }
    Ok(())
}

fn validate_skills_json(v: &Value) -> Result<(), String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "skillsJson must be a JSON array".to_string())?;
    for (i, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| format!("skillsJson[{i}] must be a JSON object"))?;
        let name = obj
            .get("skillName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("skillsJson[{i}] missing skillName"))?;
        validate_skill_name(name)?;
        if !obj.contains_key("skillContent") {
            return Err(format!("skillsJson[{i}] missing skillContent"));
        }
    }
    Ok(())
}

fn proj_work_dir(work_root: &Path, proj_id: i64) -> PathBuf {
    work_root.join(format!("proj_{proj_id}"))
}

async fn materialize_effective_config(
    db: &GatewaySessionDb,
    work_root: &Path,
    proj_id: i64,
) -> Result<bool, String> {
    let row = project_config_draft::row_for_materialize(db, proj_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no project_config for projId={proj_id}"))?;
    let work_dir = proj_work_dir(work_root, proj_id);
    tokio::fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| format!("create work dir failed: {e}"))?;
    let scaffold = gateway_global_settings::load_system_prompt_default(db)
        .await
        .map_err(|e| e.to_string())?;
    project_config_apply::apply_if_needed(&work_dir, &row, true, &scaffold)
        .await
        .map_err(|e| e.message)?;
    let applied = project_config_apply::read_applied_content_rev(&work_dir).await;
    Ok(applied.as_deref() == Some(row.content_rev.as_str()))
}

fn parse_config_put_draft_patch(args: &Value) -> Result<DraftPatch<'_>, String> {
    let mut patch = DraftPatch::default();
    let mut any = false;
    if let Some(v) = args.get("claudeMd") {
        any = true;
        let content = v
            .as_str()
            .ok_or_else(|| "claudeMd must be a string".to_string())?;
        patch.claude_md = if content.trim().is_empty() {
            ClaudeMdPatch::Clear
        } else {
            ClaudeMdPatch::Set(content)
        };
    }
    if let Some(v) = args.get("rulesJson") {
        any = true;
        validate_rules_json(v)?;
        patch.rules_json = Some(v);
    }
    if let Some(v) = args.get("mcpServersJson") {
        any = true;
        validate_mcp_servers_json(v)?;
        patch.mcp_servers_json = Some(v);
    }
    if let Some(v) = args.get("skillsJson") {
        any = true;
        validate_skills_json(v)?;
        patch.skills_json = Some(v);
    }
    if !any {
        return Err(
            "at least one of claudeMd, rulesJson, mcpServersJson, skillsJson is required".into(),
        );
    }
    Ok(patch)
}

async fn handle_tools_call(
    db: &GatewaySessionDb,
    work_root: &Path,
    nas_layout: &NasLayoutBackend,
    params: Option<Value>,
) -> Result<Value, String> {
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
            let row = row_for_editing_or_err(db, proj_id).await?;
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
        "project_config_put_draft" => {
            let patch = parse_config_put_draft_patch(&args)?;
            upsert_project_draft(db, proj_id, patch).await?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "updated": true,
                "draftOpen": true
            })))
        }
        "project_config_commit_draft" => {
            let note = args
                .get("note")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let result = project_config_draft::commit_open_draft(db, proj_id, note)
                .await
                .map_err(|e| e.message)?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "savedContentRev": result.saved_content_rev,
                "stableContentRev": result.stable_content_rev,
                "activated": false,
                "materialized": false
            })))
        }
        "project_config_activate" => {
            let content_rev = args
                .get("contentRev")
                .and_then(Value::as_str)
                .ok_or_else(|| "arguments.contentRev required".to_string())?;
            project_config_draft::activate_formal_revision(db, proj_id, content_rev)
                .await
                .map_err(|e| e.message)?;
            // FC worker reads project config from NAS `{cluster}/proj_N/home` (mounted ro as
            // `/claw_ds`): write effective config there via nas-api on activate (the real bug fix).
            // The host `work_root/proj_N` materialization is kept for now. Author: kejiqing
            let materialized = materialize_effective_config(db, work_root, proj_id).await?;
            nas_layout
                .materialize_proj_workspace(db, proj_id)
                .await
                .map_err(|e| format!("materialize project config to NAS failed: {e}"))?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "activeContentRev": content_rev,
                "activated": true,
                "materialized": materialized
            })))
        }
        "project_claude_get" => {
            let row = row_for_editing_or_err(db, proj_id).await?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "content": row.claude_md.unwrap_or_default()
            })))
        }
        "project_rules_get" => {
            let row = row_for_editing_or_err(db, proj_id).await?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "rulesJson": row.rules_json
            })))
        }
        "project_mcp_get" => {
            let row = row_for_editing_or_err(db, proj_id).await?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "mcpServersJson": row.mcp_servers_json
            })))
        }
        "project_skills_get" => {
            let row = row_for_editing_or_err(db, proj_id).await?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "skillsJson": row.skills_json
            })))
        }
        "project_claude_put_draft" => {
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| "arguments.content required".to_string())?;
            let claude_md = if content.trim().is_empty() {
                ClaudeMdPatch::Clear
            } else {
                ClaudeMdPatch::Set(content)
            };
            upsert_project_draft(
                db,
                proj_id,
                DraftPatch {
                    claude_md,
                    ..Default::default()
                },
            )
            .await?;
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "updated": true,
                "bytes": content.len()
            })))
        }
        "project_rules_put_draft" => {
            let rules_json = args
                .get("rulesJson")
                .ok_or_else(|| "arguments.rulesJson required".to_string())?;
            validate_rules_json(rules_json)?;
            upsert_project_draft(
                db,
                proj_id,
                DraftPatch {
                    rules_json: Some(rules_json),
                    ..Default::default()
                },
            )
            .await?;
            let count = rules_json.as_array().map_or(0, |a| a.len());
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "updated": true,
                "ruleCount": count
            })))
        }
        "project_mcp_put_draft" => {
            let mcp_servers_json = args
                .get("mcpServersJson")
                .ok_or_else(|| "arguments.mcpServersJson required".to_string())?;
            validate_mcp_servers_json(mcp_servers_json)?;
            upsert_project_draft(
                db,
                proj_id,
                DraftPatch {
                    mcp_servers_json: Some(mcp_servers_json),
                    ..Default::default()
                },
            )
            .await?;
            let count = mcp_servers_json.as_object().map_or(0, |o| o.len());
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "updated": true,
                "mcpServerCount": count
            })))
        }
        "project_skills_put_draft" => {
            let skills_json = args
                .get("skillsJson")
                .ok_or_else(|| "arguments.skillsJson required".to_string())?;
            validate_skills_json(skills_json)?;
            upsert_project_draft(
                db,
                proj_id,
                DraftPatch {
                    skills_json: Some(skills_json),
                    ..Default::default()
                },
            )
            .await?;
            let count = skills_json.as_array().map_or(0, |a| a.len());
            Ok(tool_text_result(&json!({
                "projId": proj_id,
                "updated": true,
                "skillCount": count
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

    const REQUIRED_TOOLS: &[&str] = &[
        "project_config_get",
        "project_config_put_draft",
        "project_config_commit_draft",
        "project_config_activate",
        "project_claude_get",
        "project_claude_put_draft",
        "project_rules_get",
        "project_rules_put_draft",
        "project_mcp_get",
        "project_mcp_put_draft",
        "project_skills_get",
        "project_skills_put_draft",
    ];

    #[test]
    fn tools_list_includes_required_project_config_tools() {
        let v = tools_list_result();
        let names: Vec<_> = v["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(Value::as_str))
            .collect();
        for expected in REQUIRED_TOOLS {
            assert!(names.contains(expected), "missing tool {expected}");
        }
        assert_eq!(names.len(), REQUIRED_TOOLS.len());
    }

    #[test]
    fn validate_skills_json_rejects_missing_skill_name() {
        let err = validate_skills_json(&json!([{ "skillContent": "x" }])).unwrap_err();
        assert!(err.contains("skillName"));
    }
}
