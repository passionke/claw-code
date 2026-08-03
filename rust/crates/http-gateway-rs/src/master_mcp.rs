//! Streamable-HTTP MCP for master observer (`POST /v1/master/{projId}/mcp`). Author: kejiqing

use axum::body::Bytes;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::admin_mcp_solve::{validate_admin_mcp_solve_input, AdminMcpSolveBackend, AdminMcpSolveInput};
use crate::gateway_admin_mcp_token::extract_bearer_token;
use crate::master_observer::{
    can_transition_repair_status, clone_stable_config_onto_project, master_mcp_shared_token,
    new_repair_run_id, promote_observation_to_apprentice_draft, replayable_inventory_items,
    validate_inventory_json, zero_pool_worker_profile_json, MasterRepairRunRow, PROJECT_ROLE_MASTER,
    REPAIR_STATUS_ANALYZED, REPAIR_STATUS_DRAFT_PUSHED, REPAIR_STATUS_OPENED,
    REPAIR_STATUS_PATCHED, REPAIR_STATUS_REPLAYED, REPAIR_STATUS_SYNCED,
};
use crate::project_config_draft;
use crate::session_db::{now_ms_for_registry, GatewaySessionDb, ProjectConfigUpsert};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_SERVER_NAME: &str = "claw-master-observer";
const MCP_SERVER_VERSION: &str = "0.1.0";

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

fn jsonrpc_result(id: Value, result: Value) -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Json(json!({"jsonrpc":"2.0","id":id,"result":result})),
    )
        .into_response()
}

fn jsonrpc_error(id: Value, code: i32, message: impl Into<String>) -> Response {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Json(json!({
            "jsonrpc":"2.0",
            "id": id,
            "error": {"code": code, "message": message.into()}
        })),
    )
        .into_response()
}

fn tool_def(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
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

fn tools_list() -> Value {
    let tools: Vec<Value> = vec![
            tool_def("apprentice_list", "List apprentices paired to this master.", json!({}), &[]),
            tool_def(
                "apprentice_config_get",
                "Read apprentice stable (effective) config package.",
                json!({"apprenticeProjId": {"type": "integer"}}),
                &["apprenticeProjId"]
            ),
            tool_def(
                "apprentice_sessions_query",
                "List sessions for an apprentice (optional time window).",
                json!({
                    "apprenticeProjId": {"type": "integer"},
                    "limit": {"type": "integer"},
                    "updatedAfterMs": {"type": "integer"},
                    "updatedBeforeMs": {"type": "integer"}
                }),
                &["apprenticeProjId"]
            ),
            tool_def(
                "apprentice_turns_list",
                "List turns for an apprentice session.",
                json!({
                    "apprenticeProjId": {"type": "integer"},
                    "sessionId": {"type": "string"}
                }),
                &["apprenticeProjId", "sessionId"]
            ),
            tool_def(
                "observation_sync_from_apprentice",
                "Clone apprentice stable config onto observation space and mark repair run synced.",
                json!({
                    "apprenticeProjId": {"type": "integer"},
                    "runId": {"type": "string"}
                }),
                &["apprenticeProjId", "runId"]
            ),
            tool_def(
                "observation_config_put_draft",
                "Write observation project draft fields (claudeMd/skillsJson/rulesJson/mcpServersJson).",
                json!({
                    "observationProjId": {"type": "integer"},
                    "runId": {"type": "string"},
                    "claudeMd": {"type": "string"},
                    "skillsJson": {},
                    "rulesJson": {},
                    "mcpServersJson": {},
                    "commitAndActivate": {"type": "boolean"}
                }),
                &["observationProjId", "runId"]
            ),
            tool_def(
                "repair_run_open",
                "Open a master_repair_run for one apprentice.",
                json!({
                    "apprenticeProjId": {"type": "integer"},
                    "masterSessionId": {"type": "string"},
                    "masterTurnId": {"type": "string"}
                }),
                &["apprenticeProjId"]
            ),
            tool_def(
                "inventory_put",
                "Replace inventory_json on an opened repair run.",
                json!({
                    "runId": {"type": "string"},
                    "inventoryJson": {}
                }),
                &["runId", "inventoryJson"]
            ),
            tool_def(
                "observation_replay",
                "Mechanically replay inventory items onto the observation project (bizdate aligned).",
                json!({
                    "runId": {"type": "string"},
                    "bizdateOverride": {"type": "string"},
                    "itemIds": {"type": "array", "items": {"type": "string"}}
                }),
                &["runId"]
            ),
            tool_def(
                "replay_results_get",
                "Read replay_session_ids and related turn summaries for a repair run.",
                json!({"runId": {"type": "string"}}),
                &["runId"]
            ),
            tool_def(
                "repair_run_analyze",
                "Write analysis_json and advance to analyzed.",
                json!({
                    "runId": {"type": "string"},
                    "analysisJson": {}
                }),
                &["runId", "analysisJson"]
            ),
            tool_def(
                "promote_to_apprentice_draft",
                "Copy observation formal config into apprentice __draft__ (does not activate).",
                json!({
                    "runId": {"type": "string"},
                    "note": {"type": "string"}
                }),
                &["runId"]
            ),
            tool_def(
                "apprentice_config_put_draft",
                "Write apprentice draft fields only (no activate).",
                json!({
                    "apprenticeProjId": {"type": "integer"},
                    "claudeMd": {"type": "string"},
                    "skillsJson": {},
                    "rulesJson": {}
                }),
                &["apprenticeProjId"]
            ),
            tool_def(
                "observation_solve",
                "Enqueue one solve_async on the observation project.",
                json!({
                    "observationProjId": {"type": "integer"},
                    "userPrompt": {"type": "string"},
                    "extraSession": {"type": "object"},
                    "sessionId": {"type": "string"}
                }),
                &["observationProjId", "userPrompt"]
            )
    ];
    json!({ "tools": tools })
}

/// Tool names advertised by tools/list (for unit tests). Author: kejiqing
#[must_use]
pub fn master_mcp_tool_names_from_schema() -> Vec<String> {
    tools_list()
        .get("tools")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect()
}

fn verify_master_mcp_auth(headers: &HeaderMap) -> Result<(), String> {
    let expected = master_mcp_shared_token()
        .ok_or_else(|| "CLAW_MASTER_MCP_TOKEN is not set on gateway".to_string())?;
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let token = extract_bearer_token(bearer).ok_or_else(|| "missing Bearer token".to_string())?;
    if token != expected {
        return Err("invalid master MCP token".into());
    }
    Ok(())
}

fn text_result(v: Value) -> Value {
    json!({
        "content": [{"type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string())}]
    })
}

pub async fn handle_master_mcp_post<B: AdminMcpSolveBackend>(
    db: &GatewaySessionDb,
    solve_backend: &B,
    master_proj_id: i64,
    headers: &HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = verify_master_mcp_auth(headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": e})),
        )
            .into_response();
    }
    let role = match db.get_project_role(master_proj_id).await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    if role != PROJECT_ROLE_MASTER {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": format!("proj {master_proj_id} is not master")})),
        )
            .into_response();
    }

    let req: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return jsonrpc_error(Value::Null, -32700, format!("parse error: {e}"));
        }
    };
    let id = req.id.unwrap_or(Value::Null);
    match req.method.as_str() {
        "initialize" => jsonrpc_result(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": MCP_SERVER_NAME, "version": MCP_SERVER_VERSION}
            }),
        ),
        "notifications/initialized" | "notifications/cancelled" => StatusCode::ACCEPTED.into_response(),
        "tools/list" => jsonrpc_result(id, tools_list()),
        "tools/call" => {
            let params = req.params.unwrap_or(json!({}));
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match dispatch_tool(db, solve_backend, master_proj_id, name, &args).await {
                Ok(v) => jsonrpc_result(id, text_result(v)),
                Err(e) => jsonrpc_result(
                    id,
                    json!({
                        "isError": true,
                        "content": [{"type": "text", "text": e}]
                    }),
                ),
            }
        }
        "ping" => jsonrpc_result(id, json!({})),
        other => jsonrpc_error(id, -32601, format!("method not found: {other}")),
    }
}

async fn dispatch_tool<B: AdminMcpSolveBackend>(
    db: &GatewaySessionDb,
    solve_backend: &B,
    master_proj_id: i64,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    match name {
        "apprentice_list" => {
            let links = db
                .list_master_links(master_proj_id)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"links": links}))
        }
        "apprentice_config_get" => {
            let aid = arg_i64(args, "apprenticeProjId")?;
            let _ = db
                .assert_master_owns_apprentice(master_proj_id, aid)
                .await?;
            let row = project_config_draft::row_for_materialize(db, aid)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("apprentice {aid} missing"))?;
            Ok(json!({
                "projId": aid,
                "stableContentRev": row.stable_content_rev,
                "claudeMd": row.claude_md,
                "skillsJson": row.skills_json,
                "rulesJson": row.rules_json,
                "mcpServersJson": row.mcp_servers_json,
                "allowedToolsJson": row.allowed_tools_json,
                "extraSessionFieldsJson": row.extra_session_fields_json
            }))
        }
        "apprentice_sessions_query" => {
            let aid = arg_i64(args, "apprenticeProjId")?;
            let _ = db
                .assert_master_owns_apprentice(master_proj_id, aid)
                .await?;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_i64())
                .unwrap_or(50)
                .clamp(1, 200);
            let after = args.get("updatedAfterMs").and_then(|v| v.as_i64());
            let before = args.get("updatedBeforeMs").and_then(|v| v.as_i64());
            let sessions = db
                .list_sessions_for_proj(aid, limit, None, None, after, before, None, None, None)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"sessions": sessions.iter().map(|s| json!({
                "sessionId": s.session_id,
                "createdAtMs": s.created_at_ms,
                "updatedAtMs": s.updated_at_ms,
                "turnCount": s.turn_count,
                "previewPrompt": s.preview_prompt,
                "clientOrigin": s.client_origin
            })).collect::<Vec<_>>()}))
        }
        "apprentice_turns_list" => {
            let aid = arg_i64(args, "apprenticeProjId")?;
            let sid = arg_str(args, "sessionId")?;
            let _ = db
                .assert_master_owns_apprentice(master_proj_id, aid)
                .await?;
            let turns = db
                .list_turns_for_session(sid, aid)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"turns": turns.iter().map(|t| json!({
                "turnId": t.turn_id,
                "userPrompt": t.user_prompt,
                "status": t.status,
                "createdAtMs": t.created_at_ms,
                "finishedAtMs": t.finished_at_ms,
                "reportBody": t.report_body,
                "failureDetail": t.failure_detail,
                "extraSession": t.extra_session,
                "feedback": t.feedback
            })).collect::<Vec<_>>()}))
        }
        "repair_run_open" => {
            let aid = arg_i64(args, "apprenticeProjId")?;
            let link = db
                .assert_master_owns_apprentice(master_proj_id, aid)
                .await?;
            let apprentice = project_config_draft::row_for_materialize(db, aid)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("apprentice {aid} missing"))?;
            let now = now_ms_for_registry();
            let run = MasterRepairRunRow {
                run_id: new_repair_run_id(),
                master_proj_id,
                apprentice_proj_id: aid,
                observation_proj_id: link.observation_proj_id,
                master_session_id: args
                    .get("masterSessionId")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                master_turn_id: args
                    .get("masterTurnId")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                status: REPAIR_STATUS_OPENED.into(),
                inventory_json: json!({"items":[]}),
                baseline_apprentice_content_rev: apprentice.stable_content_rev.clone(),
                observation_content_rev_before: None,
                observation_content_rev_after: None,
                replay_session_ids: json!([]),
                analysis_json: json!({}),
                promote_status: "none".into(),
                apprentice_draft_note: None,
                created_at_ms: now,
                updated_at_ms: now,
            };
            db.insert_repair_run(&run).await.map_err(|e| e.to_string())?;
            Ok(json!({"run": run}))
        }
        "inventory_put" => {
            let run_id = arg_str(args, "runId")?;
            let inv = args
                .get("inventoryJson")
                .cloned()
                .ok_or_else(|| "inventoryJson required".to_string())?;
            let mut run = load_owned_run(db, master_proj_id, run_id).await?;
            if run.status != REPAIR_STATUS_OPENED && run.status != REPAIR_STATUS_SYNCED {
                return Err(format!(
                    "inventory_put only allowed in opened|synced; got {}",
                    run.status
                ));
            }
            validate_inventory_json(&inv)?;
            run.inventory_json = inv;
            run.updated_at_ms = now_ms_for_registry();
            db.update_repair_run(&run).await.map_err(|e| e.to_string())?;
            Ok(json!({"run": run}))
        }
        "observation_sync_from_apprentice" => {
            let aid = arg_i64(args, "apprenticeProjId")?;
            let run_id = arg_str(args, "runId")?;
            let link = db
                .assert_master_owns_apprentice(master_proj_id, aid)
                .await?;
            let mut run = load_owned_run(db, master_proj_id, run_id).await?;
            if run.apprentice_proj_id != aid {
                return Err("run apprentice mismatch".into());
            }
            if !can_transition_repair_status(&run.status, REPAIR_STATUS_SYNCED) {
                return Err(format!("cannot sync from status {}", run.status));
            }
            let source = project_config_draft::row_for_materialize(db, aid)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("apprentice {aid} missing"))?;
            let obs = db
                .get_project_config(link.observation_proj_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "observation missing".to_string())?;
            let before = obs.stable_content_rev.clone();
            let rev = clone_stable_config_onto_project(
                db,
                &source,
                link.observation_proj_id,
                &obs.project_code,
                &obs.project_description,
                &zero_pool_worker_profile_json(),
                None,
                None,
            )
            .await?;
            run.status = REPAIR_STATUS_SYNCED.into();
            run.baseline_apprentice_content_rev = source.stable_content_rev.clone();
            run.observation_content_rev_before = before;
            run.observation_content_rev_after = Some(rev);
            run.updated_at_ms = now_ms_for_registry();
            db.update_repair_run(&run).await.map_err(|e| e.to_string())?;
            Ok(json!({"run": run}))
        }
        "observation_config_put_draft" => {
            let oid = arg_i64(args, "observationProjId")?;
            let run_id = arg_str(args, "runId")?;
            let _ = db
                .assert_master_owns_observation(master_proj_id, oid)
                .await?;
            let mut run = load_owned_run(db, master_proj_id, run_id).await?;
            if run.observation_proj_id != oid {
                return Err("run observation mismatch".into());
            }
            if run.status != REPAIR_STATUS_SYNCED
                && run.status != REPAIR_STATUS_PATCHED
                && run.status != REPAIR_STATUS_REPLAYED
                && run.status != REPAIR_STATUS_ANALYZED
            {
                return Err(format!("cannot patch from status {}", run.status));
            }
            let mut draft = project_config_draft::ensure_draft(db, oid)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(s) = args.get("claudeMd").and_then(|v| v.as_str()) {
                draft.claude_md = Some(s.to_string());
            }
            if let Some(v) = args.get("skillsJson") {
                draft.skills_json = v.clone();
            }
            if let Some(v) = args.get("rulesJson") {
                draft.rules_json = v.clone();
            }
            if let Some(v) = args.get("mcpServersJson") {
                draft.mcp_servers_json = v.clone();
            }
            draft.updated_at_ms = now_ms_for_registry();
            upsert_draft_row(db, &draft).await?;
            let commit_activate = args
                .get("commitAndActivate")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if commit_activate {
                let committed = project_config_draft::commit_open_draft(
                    db,
                    oid,
                    Some("master observation patch".into()),
                )
                .await
                .map_err(|e| e.to_string())?;
                let activated = project_config_draft::activate_formal_revision(
                    db,
                    oid,
                    &committed.saved_content_rev,
                )
                .await
                .map_err(|e| e.to_string())?;
                run.observation_content_rev_after = activated.stable_content_rev.clone();
            }
            if can_transition_repair_status(&run.status, REPAIR_STATUS_PATCHED)
                || run.status == REPAIR_STATUS_PATCHED
            {
                run.status = REPAIR_STATUS_PATCHED.into();
            }
            run.updated_at_ms = now_ms_for_registry();
            db.update_repair_run(&run).await.map_err(|e| e.to_string())?;
            Ok(json!({"run": run}))
        }
        "observation_replay" => {
            let run_id = arg_str(args, "runId")?;
            let mut run = load_owned_run(db, master_proj_id, run_id).await?;
            if !can_transition_repair_status(&run.status, REPAIR_STATUS_REPLAYED)
                && run.status != REPAIR_STATUS_PATCHED
            {
                return Err(format!("cannot replay from status {}", run.status));
            }
            if run.status != REPAIR_STATUS_PATCHED
                && run.status != REPAIR_STATUS_REPLAYED
                && run.status != REPAIR_STATUS_ANALYZED
            {
                return Err(format!("replay requires patched+; got {}", run.status));
            }
            let items = replayable_inventory_items(&run.inventory_json);
            let filter_ids: Option<std::collections::HashSet<String>> = args
                .get("itemIds")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                });
            let bizdate_override = args
                .get("bizdateOverride")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let mut replayed = run
                .replay_session_ids
                .as_array()
                .cloned()
                .unwrap_or_default();
            for item in items {
                let item_id = item
                    .get("itemId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(ref ids) = filter_ids {
                    if !ids.contains(&item_id) {
                        continue;
                    }
                }
                let src_sid = item
                    .get("sourceSessionId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("item {item_id} missing sourceSessionId"))?;
                let src_tid = item
                    .get("sourceTurnId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| format!("item {item_id} missing sourceTurnId"))?;
                let (prompt, entry) = db
                    .get_turn_for_replay(src_sid, run.apprentice_proj_id, src_tid)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("source turn {src_tid} not found"))?;
                let user_prompt = prompt.unwrap_or_default();
                if user_prompt.trim().is_empty() {
                    return Err(format!("source turn {src_tid} has empty user_prompt"));
                }
                let mut extra = entry
                    .as_ref()
                    .and_then(|v| v.get("extraSession").cloned())
                    .unwrap_or_else(|| json!({}));
                if let Some(ref bd) = bizdate_override {
                    if let Some(obj) = extra.as_object_mut() {
                        obj.insert("bizdate".into(), json!(bd));
                    }
                } else if let Some(bd) = item.get("bizdate").and_then(|v| v.as_str()) {
                    if let Some(obj) = extra.as_object_mut() {
                        obj.insert("bizdate".into(), json!(bd));
                    }
                }
                let input = AdminMcpSolveInput {
                    proj_id: run.observation_proj_id,
                    user_prompt: user_prompt.clone(),
                    session_id: None,
                    model: None,
                    timeout_seconds: None,
                    extra_session: Some(extra.clone()),
                    allowed_tools: None,
                    max_iterations: None,
                    attachments: None,
                };
                validate_admin_mcp_solve_input(db, &input).await?;
                let resp = solve_backend
                    .gateway_solve_async(input)
                    .await
                    .map_err(|e| e.to_string())?;
                replayed.push(json!({
                    "itemId": item_id,
                    "sourceSessionId": src_sid,
                    "sourceTurnId": src_tid,
                    "observationSessionId": resp.get("sessionId").cloned().unwrap_or(Value::Null),
                    "observationTurnId": resp.get("turnId").cloned().unwrap_or(Value::Null),
                    "taskId": resp.get("taskId").cloned().unwrap_or(Value::Null),
                    "userPrompt": user_prompt,
                    "extraSession": extra
                }));
            }
            run.replay_session_ids = Value::Array(replayed);
            run.status = REPAIR_STATUS_REPLAYED.into();
            run.updated_at_ms = now_ms_for_registry();
            db.update_repair_run(&run).await.map_err(|e| e.to_string())?;
            Ok(json!({"run": run}))
        }
        "replay_results_get" => {
            let run_id = arg_str(args, "runId")?;
            let run = load_owned_run(db, master_proj_id, run_id).await?;
            Ok(json!({
                "runId": run.run_id,
                "status": run.status,
                "replaySessionIds": run.replay_session_ids,
                "analysisJson": run.analysis_json
            }))
        }
        "repair_run_analyze" => {
            let run_id = arg_str(args, "runId")?;
            let analysis = args
                .get("analysisJson")
                .cloned()
                .ok_or_else(|| "analysisJson required".to_string())?;
            let mut run = load_owned_run(db, master_proj_id, run_id).await?;
            if !can_transition_repair_status(&run.status, REPAIR_STATUS_ANALYZED) {
                return Err(format!("cannot analyze from status {}", run.status));
            }
            run.analysis_json = analysis;
            run.status = REPAIR_STATUS_ANALYZED.into();
            run.updated_at_ms = now_ms_for_registry();
            db.update_repair_run(&run).await.map_err(|e| e.to_string())?;
            Ok(json!({"run": run}))
        }
        "promote_to_apprentice_draft" => {
            let run_id = arg_str(args, "runId")?;
            let note = args
                .get("note")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("promoted from repair run {run_id}"));
            let mut run = load_owned_run(db, master_proj_id, run_id).await?;
            if !can_transition_repair_status(&run.status, REPAIR_STATUS_DRAFT_PUSHED) {
                return Err(format!("cannot promote from status {}", run.status));
            }
            promote_observation_to_apprentice_draft(
                db,
                run.observation_proj_id,
                run.apprentice_proj_id,
                &note,
            )
            .await?;
            run.status = REPAIR_STATUS_DRAFT_PUSHED.into();
            run.promote_status = "draft_pushed".into();
            run.apprentice_draft_note = Some(note);
            run.updated_at_ms = now_ms_for_registry();
            db.update_repair_run(&run).await.map_err(|e| e.to_string())?;
            Ok(json!({"run": run}))
        }
        "apprentice_config_put_draft" => {
            let aid = arg_i64(args, "apprenticeProjId")?;
            let _ = db
                .assert_master_owns_apprentice(master_proj_id, aid)
                .await?;
            let mut draft = project_config_draft::ensure_draft(db, aid)
                .await
                .map_err(|e| e.to_string())?;
            if let Some(s) = args.get("claudeMd").and_then(|v| v.as_str()) {
                draft.claude_md = Some(s.to_string());
            }
            if let Some(v) = args.get("skillsJson") {
                draft.skills_json = v.clone();
            }
            if let Some(v) = args.get("rulesJson") {
                draft.rules_json = v.clone();
            }
            draft.updated_at_ms = now_ms_for_registry();
            upsert_draft_row(db, &draft).await?;
            Ok(json!({"projId": aid, "draftOpen": true}))
        }
        "observation_solve" => {
            let oid = arg_i64(args, "observationProjId")?;
            let _ = db
                .assert_master_owns_observation(master_proj_id, oid)
                .await?;
            let prompt = arg_str(args, "userPrompt")?;
            let input = AdminMcpSolveInput {
                proj_id: oid,
                user_prompt: prompt.to_string(),
                session_id: args
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                model: None,
                timeout_seconds: None,
                extra_session: args.get("extraSession").cloned(),
                allowed_tools: None,
                max_iterations: None,
                attachments: None,
            };
            validate_admin_mcp_solve_input(db, &input).await?;
            let resp = solve_backend
                .gateway_solve_async(input)
                .await
                .map_err(|e| e.to_string())?;
            Ok(resp)
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

async fn load_owned_run(
    db: &GatewaySessionDb,
    master_proj_id: i64,
    run_id: &str,
) -> Result<MasterRepairRunRow, String> {
    let run = db
        .get_repair_run(run_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("run {run_id} not found"))?;
    if run.master_proj_id != master_proj_id {
        return Err("run not owned by this master".into());
    }
    Ok(run)
}

async fn upsert_draft_row(
    db: &GatewaySessionDb,
    draft: &crate::session_db::ProjectConfigRow,
) -> Result<(), String> {
    use crate::project_config_draft::DRAFT_CONTENT_REV;
    db.upsert_project_config(ProjectConfigUpsert {
        proj_id: draft.proj_id,
        content_rev: DRAFT_CONTENT_REV,
        stable_content_rev: draft.stable_content_rev.as_deref(),
        draft_open: true,
        updated_at_ms: draft.updated_at_ms,
        rules_json: &draft.rules_json,
        mcp_servers_json: &draft.mcp_servers_json,
        skills_sources_json: &draft.skills_sources_json,
        skills_json: &draft.skills_json,
        allowed_tools_json: &draft.allowed_tools_json,
        claude_md: draft.claude_md.as_deref(),
        git_sync_json: &draft.git_sync_json,
        solve_preflight_json: &draft.solve_preflight_json,
        solve_orchestration_json: &draft.solve_orchestration_json,
        language_pipeline_json: &draft.language_pipeline_json,
        extra_session_fields_json: &draft.extra_session_fields_json,
        prompt_limits_json: &draft.prompt_limits_json,
        worker_profile_json: &draft.worker_profile_json,
        worker_env_json: &draft.worker_env_json,
        project_code: &draft.project_code,
        project_description: &draft.project_description,
        max_iterations: draft.max_iterations,
    })
    .await
    .map_err(|e| e.to_string())
}

fn arg_i64(args: &Value, key: &str) -> Result<i64, String> {
    args.get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| format!("{key} required"))
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{key} required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use crate::master_observer::MASTER_MCP_TOOL_NAMES;

    #[test]
    fn tools_list_matches_contract_names() {
        let names = master_mcp_tool_names_from_schema();
        assert_eq!(names.len(), MASTER_MCP_TOOL_NAMES.len());
        for expected in MASTER_MCP_TOOL_NAMES {
            assert!(
                names.iter().any(|n| n == expected),
                "missing tool {expected}"
            );
        }
    }

    #[test]
    fn verify_auth_requires_env_and_bearer() {
        let _guard = crate::pool::test_env_lock();
        let prev = std::env::var("CLAW_MASTER_MCP_TOKEN").ok();
        std::env::remove_var("CLAW_MASTER_MCP_TOKEN");
        let headers = HeaderMap::new();
        assert!(verify_master_mcp_auth(&headers)
            .unwrap_err()
            .contains("CLAW_MASTER_MCP_TOKEN"));
        std::env::set_var("CLAW_MASTER_MCP_TOKEN", "abc");
        assert!(verify_master_mcp_auth(&headers)
            .unwrap_err()
            .contains("missing Bearer"));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer wrong"),
        );
        assert!(verify_master_mcp_auth(&headers)
            .unwrap_err()
            .contains("invalid"));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc"),
        );
        assert!(verify_master_mcp_auth(&headers).is_ok());
        match prev {
            Some(v) => std::env::set_var("CLAW_MASTER_MCP_TOKEN", v),
            None => std::env::remove_var("CLAW_MASTER_MCP_TOKEN"),
        }
    }
}
