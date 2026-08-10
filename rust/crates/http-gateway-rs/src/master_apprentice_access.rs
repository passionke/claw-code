//! Local vs remote apprentice access for master observer. Author: kejiqing
//!
//! Empty / self `apprentice_gateway_base` → local PG.
//! Otherwise → HTTP peer API using **per-link** `apprentice_mcp_token`
//! (that peer's `CLAW_MASTER_MCP_TOKEN`; clusters need not share one token).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::gateway_admin_mcp_token::extract_bearer_token;
use crate::gateway_endpoint::{normalize_gateway_base, parse_apprentice_gateway_base};
use crate::master_observer::{
    master_mcp_shared_token, ProjectMasterLinkRow, PROJECT_ROLE_MASTER, PROJECT_ROLE_OBSERVATION,
};
use crate::project_config_draft::{self, DRAFT_CONTENT_REV};
use crate::session_db::{
    now_ms_for_registry, GatewaySessionDb, ProjectConfigRow, ProjectConfigUpsert,
};

/// Wire DTO for peer stable-config + local conversion. Author: kejiqing
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprenticeStableConfigDto {
    pub proj_id: i64,
    pub project_role: String,
    pub stable_content_rev: Option<String>,
    pub claude_md: Option<String>,
    pub skills_json: Value,
    pub rules_json: Value,
    pub mcp_servers_json: Value,
    pub allowed_tools_json: Value,
    pub solve_preflight_json: Value,
    pub solve_orchestration_json: Value,
    pub language_pipeline_json: Value,
    pub extra_session_fields_json: Value,
    pub prompt_limits_json: Value,
    pub worker_env_json: Value,
    pub max_iterations: Option<usize>,
    pub project_code: String,
    pub project_description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApprenticeDraftPutDto {
    pub claude_md: Option<String>,
    #[schema(value_type = Object)]
    pub skills_json: Option<Value>,
    #[schema(value_type = Object)]
    pub rules_json: Option<Value>,
    #[schema(value_type = Object)]
    pub mcp_servers_json: Option<Value>,
    #[schema(value_type = Object)]
    pub allowed_tools_json: Option<Value>,
}

#[must_use]
pub fn link_peer_base(link: &ProjectMasterLinkRow, self_gateway_base: &str) -> Option<String> {
    let raw = link.apprentice_gateway_base.trim();
    if raw.is_empty() {
        return None;
    }
    let Ok(peer) = parse_apprentice_gateway_base(raw) else {
        return Some(raw.trim_end_matches('/').to_string());
    };
    if peer.is_empty() {
        return None;
    }
    let self_n = normalize_gateway_base(self_gateway_base)
        .unwrap_or_else(|_| self_gateway_base.trim().trim_end_matches('/').to_string());
    if peer == self_n {
        None
    } else {
        Some(peer)
    }
}

pub fn row_to_stable_dto(row: &ProjectConfigRow, project_role: &str) -> ApprenticeStableConfigDto {
    ApprenticeStableConfigDto {
        proj_id: row.proj_id,
        project_role: project_role.to_string(),
        stable_content_rev: row.stable_content_rev.clone(),
        claude_md: row.claude_md.clone(),
        skills_json: row.skills_json.clone(),
        rules_json: row.rules_json.clone(),
        mcp_servers_json: row.mcp_servers_json.clone(),
        allowed_tools_json: row.allowed_tools_json.clone(),
        solve_preflight_json: row.solve_preflight_json.clone(),
        solve_orchestration_json: row.solve_orchestration_json.clone(),
        language_pipeline_json: row.language_pipeline_json.clone(),
        extra_session_fields_json: row.extra_session_fields_json.clone(),
        prompt_limits_json: row.prompt_limits_json.clone(),
        worker_env_json: row.worker_env_json.clone(),
        max_iterations: row.max_iterations,
        project_code: row.project_code.clone(),
        project_description: row.project_description.clone(),
    }
}

pub fn dto_to_config_row(dto: &ApprenticeStableConfigDto) -> ProjectConfigRow {
    let rev = dto
        .stable_content_rev
        .clone()
        .unwrap_or_else(|| "remote-stable".into());
    ProjectConfigRow {
        proj_id: dto.proj_id,
        content_rev: rev.clone(),
        stable_content_rev: Some(rev),
        draft_open: false,
        updated_at_ms: now_ms_for_registry(),
        rules_json: dto.rules_json.clone(),
        mcp_servers_json: dto.mcp_servers_json.clone(),
        skills_sources_json: json!([]),
        skills_json: dto.skills_json.clone(),
        allowed_tools_json: dto.allowed_tools_json.clone(),
        claude_md: dto.claude_md.clone(),
        git_sync_json: json!({}),
        solve_preflight_json: dto.solve_preflight_json.clone(),
        solve_orchestration_json: dto.solve_orchestration_json.clone(),
        language_pipeline_json: dto.language_pipeline_json.clone(),
        extra_session_fields_json: dto.extra_session_fields_json.clone(),
        prompt_limits_json: dto.prompt_limits_json.clone(),
        worker_profile_json: json!({}),
        worker_env_json: dto.worker_env_json.clone(),
        project_code: dto.project_code.clone(),
        project_description: dto.project_description.clone(),
        max_iterations: dto.max_iterations,
    }
}

/// Bearer for peer calls: per-link token (remote gateway's master MCP token). Author: kejiqing
pub fn peer_auth_token(link: &ProjectMasterLinkRow) -> Result<String, String> {
    let t = link.apprentice_mcp_token.trim();
    if t.is_empty() {
        return Err(
            "remote apprentice requires mcpToken (peer gateway CLAW_MASTER_MCP_TOKEN)".into(),
        );
    }
    Ok(t.to_string())
}

async fn peer_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("peer http client: {e}"))
}

async fn peer_get_json(base: &str, token: &str, path: &str) -> Result<Value, String> {
    let url = format!("{}{path}", base.trim_end_matches('/'));
    let resp = peer_client()
        .await?
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("peer GET {url}: {e}"))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("peer GET body: {e}"))?;
    if !status.is_success() {
        return Err(format!("peer GET {url} → {status}: {body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("peer GET json: {e}; body={body}"))
}

async fn peer_post_json(
    base: &str,
    token: &str,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    let url = format!("{}{path}", base.trim_end_matches('/'));
    let resp = peer_client()
        .await?
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .json(body)
        .send()
        .await
        .map_err(|e| format!("peer POST {url}: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("peer POST body: {e}"))?;
    if !status.is_success() {
        return Err(format!("peer POST {url} → {status}: {text}"));
    }
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).map_err(|e| format!("peer POST json: {e}; body={text}"))
}

async fn peer_put_json(base: &str, token: &str, path: &str, body: &Value) -> Result<Value, String> {
    let url = format!("{}{path}", base.trim_end_matches('/'));
    let resp = peer_client()
        .await?
        .put(&url)
        .header("Authorization", format!("Bearer {token}"))
        .json(body)
        .send()
        .await
        .map_err(|e| format!("peer PUT {url}: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("peer PUT body: {e}"))?;
    if !status.is_success() {
        return Err(format!("peer PUT {url} → {status}: {text}"));
    }
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).map_err(|e| format!("peer PUT json: {e}; body={text}"))
}

/// Ask apprentice's gateway to create the shadow observation project. Author: kejiqing
pub async fn create_observation_on_peer(
    peer_base: &str,
    peer_token: &str,
    master_proj_id: i64,
    apprentice_proj_id: i64,
) -> Result<i64, String> {
    let v = peer_post_json(
        peer_base,
        peer_token,
        &format!("/v1/master-peer/projects/{apprentice_proj_id}/observation"),
        &json!({ "masterProjId": master_proj_id }),
    )
    .await?;
    v.get("observationProjId")
        .and_then(|x| x.as_i64())
        .ok_or_else(|| format!("peer create observation missing observationProjId: {v}"))
}

pub async fn load_observation_stable(
    db: &GatewaySessionDb,
    self_gateway_base: &str,
    link: &ProjectMasterLinkRow,
) -> Result<ApprenticeStableConfigDto, String> {
    let oid = link.observation_proj_id;
    if let Some(peer) = link_peer_base(link, self_gateway_base) {
        let token = peer_auth_token(link)?;
        let v = peer_get_json(
            &peer,
            &token,
            &format!("/v1/master-peer/projects/{oid}/stable-config"),
        )
        .await?;
        return serde_json::from_value(v).map_err(|e| format!("peer obs stable decode: {e}"));
    }
    let role = db.get_project_role(oid).await.map_err(|e| e.to_string())?;
    let row = project_config_draft::row_for_materialize(db, oid)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("observation {oid} has no stable config"))?;
    Ok(row_to_stable_dto(&row, &role))
}

/// Sync apprentice stable → observation (both on apprentice gateway). Author: kejiqing
pub async fn sync_observation_from_apprentice(
    db: &GatewaySessionDb,
    self_gateway_base: &str,
    link: &ProjectMasterLinkRow,
) -> Result<(Option<String>, String, Option<String>), String> {
    let aid = link.apprentice_proj_id;
    let oid = link.observation_proj_id;
    if let Some(peer) = link_peer_base(link, self_gateway_base) {
        let token = peer_auth_token(link)?;
        let v = peer_post_json(
            &peer,
            &token,
            &format!("/v1/master-peer/observations/{oid}/sync-from/{aid}"),
            &json!({}),
        )
        .await?;
        let before = v
            .get("beforeContentRev")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        let after = v
            .get("afterContentRev")
            .and_then(|x| x.as_str())
            .ok_or_else(|| format!("peer sync missing afterContentRev: {v}"))?
            .to_string();
        let baseline = v
            .get("baselineApprenticeContentRev")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        return Ok((before, after, baseline));
    }
    let source = project_config_draft::row_for_materialize(db, aid)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("apprentice {aid} missing"))?;
    let obs = db
        .get_project_config(oid)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "observation missing".to_string())?;
    let before = obs.stable_content_rev.clone();
    let rev = crate::master_observer::clone_stable_config_onto_project(
        db,
        &source,
        oid,
        &obs.project_code,
        &obs.project_description,
        &crate::master_observer::zero_pool_worker_profile_json(),
        None,
        None,
    )
    .await?;
    Ok((before, rev, source.stable_content_rev.clone()))
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ObservationDraftPutDto {
    pub claude_md: Option<String>,
    #[schema(value_type = Object)]
    pub skills_json: Option<Value>,
    #[schema(value_type = Object)]
    pub rules_json: Option<Value>,
    #[schema(value_type = Object)]
    pub mcp_servers_json: Option<Value>,
    #[serde(default = "default_true")]
    pub commit_and_activate: bool,
}

fn default_true() -> bool {
    true
}

pub async fn put_observation_draft(
    db: &GatewaySessionDb,
    self_gateway_base: &str,
    link: &ProjectMasterLinkRow,
    patch: &ObservationDraftPutDto,
) -> Result<Option<String>, String> {
    let oid = link.observation_proj_id;
    if let Some(peer) = link_peer_base(link, self_gateway_base) {
        let token = peer_auth_token(link)?;
        let body = serde_json::to_value(patch).map_err(|e| e.to_string())?;
        let v = peer_put_json(
            &peer,
            &token,
            &format!("/v1/master-peer/observations/{oid}/draft"),
            &body,
        )
        .await?;
        return Ok(v
            .get("stableContentRev")
            .and_then(|x| x.as_str())
            .map(str::to_string));
    }
    apply_observation_draft_local(db, oid, patch).await
}

pub async fn apply_observation_draft_local(
    db: &GatewaySessionDb,
    observation_proj_id: i64,
    patch: &ObservationDraftPutDto,
) -> Result<Option<String>, String> {
    apply_draft_patch_local(
        db,
        observation_proj_id,
        &ApprenticeDraftPutDto {
            claude_md: patch.claude_md.clone(),
            skills_json: patch.skills_json.clone(),
            rules_json: patch.rules_json.clone(),
            mcp_servers_json: patch.mcp_servers_json.clone(),
            allowed_tools_json: None,
        },
    )
    .await?;
    if !patch.commit_and_activate {
        return Ok(None);
    }
    let committed = project_config_draft::commit_open_draft(
        db,
        observation_proj_id,
        Some("master observation patch".into()),
    )
    .await
    .map_err(|e| e.to_string())?;
    let activated = project_config_draft::activate_formal_revision(
        db,
        observation_proj_id,
        &committed.saved_content_rev,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(activated.stable_content_rev)
}

/// Enqueue solve on observation project via peer (remote only). Author: kejiqing
pub async fn solve_observation_on_peer(
    peer_base: &str,
    peer_token: &str,
    observation_proj_id: i64,
    user_prompt: &str,
    session_id: Option<String>,
    extra_session: Option<Value>,
) -> Result<Value, String> {
    peer_post_json(
        peer_base,
        peer_token,
        &format!("/v1/master-peer/observations/{observation_proj_id}/solve"),
        &json!({
            "userPrompt": user_prompt,
            "sessionId": session_id,
            "extraSession": extra_session,
        }),
    )
    .await
}

pub async fn load_apprentice_stable(
    db: &GatewaySessionDb,
    self_gateway_base: &str,
    link: &ProjectMasterLinkRow,
) -> Result<ApprenticeStableConfigDto, String> {
    let aid = link.apprentice_proj_id;
    if let Some(peer) = link_peer_base(link, self_gateway_base) {
        let token = peer_auth_token(link)?;
        let v = peer_get_json(
            &peer,
            &token,
            &format!("/v1/master-peer/projects/{aid}/stable-config"),
        )
        .await?;
        return serde_json::from_value(v).map_err(|e| format!("peer stable-config decode: {e}"));
    }
    let role = db.get_project_role(aid).await.map_err(|e| e.to_string())?;
    let row = project_config_draft::row_for_materialize(db, aid)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("apprentice {aid} has no stable config"))?;
    Ok(row_to_stable_dto(&row, &role))
}

pub async fn assert_apprentice_pairable(
    db: &GatewaySessionDb,
    self_gateway_base: &str,
    apprentice_proj_id: i64,
    gateway_base: &str,
    mcp_token: &str,
) -> Result<ApprenticeStableConfigDto, String> {
    let mut link = ProjectMasterLinkRow {
        master_proj_id: 0,
        apprentice_proj_id,
        observation_proj_id: 0,
        apprentice_gateway_base: gateway_base.to_string(),
        apprentice_mcp_token: mcp_token.to_string(),
        mcp_token_set: !mcp_token.trim().is_empty(),
        orphaned: false,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    // Normalize stored form before routing. Author: kejiqing
    link.apprentice_gateway_base = parse_apprentice_gateway_base(gateway_base)?;
    if link_peer_base(&link, self_gateway_base).is_some()
        && link.apprentice_mcp_token.trim().is_empty()
    {
        return Err("mcpToken is required when gatewayBase points to another gateway".into());
    }
    let dto = load_apprentice_stable(db, self_gateway_base, &link).await?;
    if dto.project_role == PROJECT_ROLE_MASTER || dto.project_role == PROJECT_ROLE_OBSERVATION {
        return Err(format!(
            "proj {apprentice_proj_id} cannot be an apprentice (role={})",
            dto.project_role
        ));
    }
    Ok(dto)
}

pub async fn list_apprentice_sessions(
    db: &GatewaySessionDb,
    self_gateway_base: &str,
    link: &ProjectMasterLinkRow,
    limit: i64,
    after: Option<i64>,
    before: Option<i64>,
) -> Result<Value, String> {
    let aid = link.apprentice_proj_id;
    if let Some(peer) = link_peer_base(link, self_gateway_base) {
        let token = peer_auth_token(link)?;
        let mut q = format!("/v1/master-peer/projects/{aid}/sessions?limit={limit}");
        if let Some(a) = after {
            q.push_str(&format!("&updatedAfterMs={a}"));
        }
        if let Some(b) = before {
            q.push_str(&format!("&updatedBeforeMs={b}"));
        }
        return peer_get_json(&peer, &token, &q).await;
    }
    let sessions = db
        .list_sessions_for_proj(aid, limit, None, None, after, before, None, None, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "sessions": sessions.iter().map(|s| json!({
            "sessionId": s.session_id,
            "createdAtMs": s.created_at_ms,
            "updatedAtMs": s.updated_at_ms,
            "turnCount": s.turn_count,
            "previewPrompt": s.preview_prompt,
            "clientOrigin": s.client_origin
        })).collect::<Vec<_>>()
    }))
}

pub async fn list_apprentice_turns(
    db: &GatewaySessionDb,
    self_gateway_base: &str,
    link: &ProjectMasterLinkRow,
    session_id: &str,
) -> Result<Value, String> {
    let aid = link.apprentice_proj_id;
    if let Some(peer) = link_peer_base(link, self_gateway_base) {
        let token = peer_auth_token(link)?;
        let path = format!(
            "/v1/master-peer/projects/{aid}/sessions/{}/turns",
            urlencoding_path(session_id)
        );
        return peer_get_json(&peer, &token, &path).await;
    }
    let turns = db
        .list_turns_for_session(session_id, aid)
        .await
        .map_err(|e| e.to_string())?;
    Ok(json!({
        "turns": turns.iter().map(|t| json!({
            "turnId": t.turn_id,
            "userPrompt": t.user_prompt,
            "status": t.status,
            "createdAtMs": t.created_at_ms,
            "finishedAtMs": t.finished_at_ms,
            "reportBody": t.report_body,
            "failureDetail": t.failure_detail,
            "extraSession": t.extra_session,
            "feedback": t.feedback
        })).collect::<Vec<_>>()
    }))
}

pub async fn get_apprentice_turn_for_replay(
    db: &GatewaySessionDb,
    self_gateway_base: &str,
    link: &ProjectMasterLinkRow,
    session_id: &str,
    turn_id: &str,
) -> Result<Option<(Option<String>, Option<Value>)>, String> {
    let aid = link.apprentice_proj_id;
    if let Some(peer) = link_peer_base(link, self_gateway_base) {
        let token = peer_auth_token(link)?;
        let path = format!(
            "/v1/master-peer/projects/{aid}/replay-turn?sessionId={}&turnId={}",
            urlencoding_query(session_id),
            urlencoding_query(turn_id)
        );
        let v = peer_get_json(&peer, &token, &path).await?;
        if v.get("found").and_then(|x| x.as_bool()) == Some(false) {
            return Ok(None);
        }
        let prompt = v
            .get("userPrompt")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        let entry = v.get("entryParams").cloned();
        return Ok(Some((prompt, entry)));
    }
    db.get_turn_for_replay(session_id, aid, turn_id)
        .await
        .map_err(|e| e.to_string())
}

pub async fn put_apprentice_draft(
    db: &GatewaySessionDb,
    self_gateway_base: &str,
    link: &ProjectMasterLinkRow,
    patch: &ApprenticeDraftPutDto,
) -> Result<(), String> {
    let aid = link.apprentice_proj_id;
    if let Some(peer) = link_peer_base(link, self_gateway_base) {
        let token = peer_auth_token(link)?;
        let body = serde_json::to_value(patch).map_err(|e| e.to_string())?;
        let _ = peer_put_json(
            &peer,
            &token,
            &format!("/v1/master-peer/projects/{aid}/draft"),
            &body,
        )
        .await?;
        return Ok(());
    }
    apply_draft_patch_local(db, aid, patch).await
}

pub async fn apply_draft_patch_local(
    db: &GatewaySessionDb,
    apprentice_proj_id: i64,
    patch: &ApprenticeDraftPutDto,
) -> Result<(), String> {
    let mut draft = project_config_draft::ensure_draft(db, apprentice_proj_id)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(ref s) = patch.claude_md {
        draft.claude_md = Some(s.clone());
    }
    if let Some(ref v) = patch.skills_json {
        draft.skills_json = v.clone();
    }
    if let Some(ref v) = patch.rules_json {
        draft.rules_json = v.clone();
    }
    if let Some(ref v) = patch.mcp_servers_json {
        draft.mcp_servers_json = v.clone();
    }
    if let Some(ref v) = patch.allowed_tools_json {
        draft.allowed_tools_json = v.clone();
    }
    draft.content_rev = DRAFT_CONTENT_REV.to_string();
    draft.draft_open = true;
    draft.updated_at_ms = now_ms_for_registry();
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
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn verify_master_peer_auth(headers: &axum::http::HeaderMap) -> Result<(), String> {
    let expected = master_mcp_shared_token()
        .ok_or_else(|| "CLAW_MASTER_MCP_TOKEN is not set on gateway".to_string())?;
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    let token = extract_bearer_token(bearer).ok_or_else(|| "missing Bearer token".to_string())?;
    if token != expected {
        return Err("invalid master peer token".into());
    }
    Ok(())
}

fn urlencoding_path(s: &str) -> String {
    // session ids are typically URL-safe; still escape reserved chars. Author: kejiqing
    s.replace('/', "%2F")
}

fn urlencoding_query(s: &str) -> String {
    urlencoding_path(s)
        .replace('?', "%3F")
        .replace('&', "%26")
        .replace('=', "%3D")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_peer_base_treats_empty_and_self_as_local() {
        let link = ProjectMasterLinkRow {
            master_proj_id: 1,
            apprentice_proj_id: 2,
            observation_proj_id: 3,
            apprentice_gateway_base: String::new(),
            apprentice_mcp_token: String::new(),
            mcp_token_set: false,
            orphaned: false,
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        assert!(link_peer_base(&link, "http://10.0.0.1:18088").is_none());
        let mut remote = link.clone();
        remote.apprentice_gateway_base = "http://10.0.0.1:18088/".into();
        assert!(link_peer_base(&remote, "http://10.0.0.1:18088").is_none());
        remote.apprentice_gateway_base = "10.0.0.2".into();
        remote.apprentice_mcp_token = "peer-secret".into();
        assert_eq!(
            link_peer_base(&remote, "http://10.0.0.1:18088").as_deref(),
            Some("http://10.0.0.2:18088")
        );
        assert_eq!(peer_auth_token(&remote).unwrap(), "peer-secret");
        remote.apprentice_mcp_token.clear();
        assert!(peer_auth_token(&remote).is_err());
    }
}
