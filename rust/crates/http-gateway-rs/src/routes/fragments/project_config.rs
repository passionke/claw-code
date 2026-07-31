// Fragment of routes::app (include!). Author: kejiqing

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct UpsertProjectConfigRequest {
    #[serde(rename = "contentRev", default)]
    content_rev: String,
    #[serde(rename = "rulesJson", default)]
    #[schema(value_type = Object)]
    rules_json: Value,
    #[serde(rename = "mcpServersJson", default)]
    #[schema(value_type = Object)]
    mcp_servers_json: Value,
    #[serde(rename = "skillsSourcesJson", default)]
    #[schema(value_type = Object)]
    skills_sources_json: Value,
    #[serde(rename = "skillsJson", default)]
    #[schema(value_type = Object)]
    skills_json: Value,
    #[serde(rename = "allowedToolsJson", default)]
    #[schema(value_type = Object)]
    allowed_tools_json: Value,
    #[serde(rename = "claudeMd")]
    claude_md: Option<String>,
    /// Omit on PUT to keep existing `git_sync_json`. Author: kejiqing
    #[serde(rename = "gitSyncJson", default)]
    git_sync_json: Option<Value>,
    /// Omit on PUT to keep existing `solve_preflight_json`. Author: kejiqing
    #[serde(rename = "solvePreflightJson", default)]
    solve_preflight_json: Option<Value>,
    /// Omit on PUT to keep existing `solve_orchestration_json`. Author: kejiqing
    #[serde(rename = "solveOrchestrationJson", default)]
    solve_orchestration_json: Option<Value>,
    /// Omit on PUT to keep existing `language_pipeline_json`. Author: kejiqing
    #[serde(rename = "languagePipelineJson", default)]
    language_pipeline_json: Option<Value>,
    /// Omit on PUT to keep existing `extra_session_fields_json`. Author: kejiqing
    #[serde(rename = "extraSessionFieldsJson", default)]
    extra_session_fields_json: Option<Value>,
    /// Omit on PUT to keep existing `prompt_limits_json`. Author: kejiqing
    #[serde(rename = "promptLimitsJson", default)]
    prompt_limits_json: Option<Value>,
    /// Omit on PUT to keep existing `worker_profile_json`. Author: kejiqing
    #[serde(rename = "workerProfileJson", default)]
    worker_profile_json: Option<Value>,
    /// Omit to keep; JSON `null` clears; positive int sets. Author: kejiqing
    #[serde(default, rename = "maxIterations")]
    #[allow(clippy::option_option)]
    max_iterations: Option<Option<usize>>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct CommitProjectConfigDraftRequest {
    /// Optional label; version id is auto-generated (`YYYYMMDDHHmmss` local). Author: kejiqing
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ProjectConfigResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "contentRev")]
    content_rev: String,
    #[serde(rename = "stableContentRev", skip_serializing_if = "Option::is_none")]
    stable_content_rev: Option<String>,
    #[serde(rename = "draftOpen")]
    draft_open: bool,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: i64,
    #[serde(rename = "rulesJson")]
    #[schema(value_type = Object)]
    rules_json: Value,
    #[serde(rename = "mcpServersJson")]
    #[schema(value_type = Object)]
    mcp_servers_json: Value,
    #[serde(rename = "skillsSourcesJson")]
    #[schema(value_type = Object)]
    skills_sources_json: Value,
    #[serde(rename = "skillsJson")]
    #[schema(value_type = Object)]
    skills_json: Value,
    #[serde(rename = "allowedToolsJson")]
    #[schema(value_type = Object)]
    allowed_tools_json: Value,
    #[serde(rename = "claudeMd")]
    claude_md: Option<String>,
    #[serde(rename = "gitSyncJson")]
    #[schema(value_type = Object)]
    git_sync_json: Value,
    #[serde(rename = "solvePreflightJson")]
    #[schema(value_type = Object)]
    solve_preflight_json: Value,
    #[serde(rename = "solveOrchestrationJson")]
    #[schema(value_type = Object)]
    solve_orchestration_json: Value,
    #[serde(rename = "languagePipelineJson")]
    #[schema(value_type = Object)]
    language_pipeline_json: Value,
    #[serde(rename = "extraSessionFieldsJson")]
    #[schema(value_type = Object)]
    extra_session_fields_json: Value,
    #[serde(rename = "promptLimitsJson")]
    #[schema(value_type = Object)]
    prompt_limits_json: Value,
    #[serde(rename = "workerProfileJson")]
    #[schema(value_type = Object)]
    worker_profile_json: Value,
    #[serde(rename = "projectCode")]
    project_code: String,
    #[serde(rename = "projectDescription")]
    project_description: String,
    /// Project default agent loop max iterations; omit/null = cluster default. Author: kejiqing
    #[serde(rename = "maxIterations")]
    max_iterations: Option<usize>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ProjectConfigVersionsResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    /// Effective formal revision id (one of non-draft rows in `versions`).
    #[serde(rename = "activeContentRev")]
    active_content_rev: String,
    #[serde(rename = "appliedContentRev", skip_serializing_if = "Option::is_none")]
    applied_content_rev: Option<String>,
    #[serde(rename = "draftOpen")]
    draft_open: bool,
    /// Formal revisions plus optional single `__draft__` row when `draftOpen`.
    versions: Vec<ProjectConfigVersionEntry>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ProjectConfigVersionEntry {
    #[serde(rename = "contentRev")]
    content_rev: String,
    #[serde(rename = "createdAtMs")]
    created_at_ms: i64,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "note", skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(rename = "isActive")]
    is_active: bool,
    #[serde(rename = "claudeInDb")]
    claude_in_db: bool,
    #[serde(rename = "skillsCountDb")]
    skills_count_db: i64,
    #[serde(rename = "rulesCountDb")]
    rules_count_db: i64,
    #[serde(rename = "mcpServersCountDb")]
    mcp_servers_count_db: i64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct CompareProjectConfigQuery {
    from: String,
    to: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ActivateProjectConfigVersionResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "activeContentRev")]
    active_content_rev: String,
    activated: bool,
    #[serde(rename = "materialized")]
    materialized: bool,
}

#[utoipa::path(
    get,
    path = "/v1/project/config/{proj_id}/entities/{domain}/{entity_key}/versions",
    tag = "ProjectConfig",
    operation_id = "list_project_entity_versions",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("domain" = project_entity_revision::ProjectEntityDomain, Path, description = "Entity domain"),
        ("entity_key" = String, Path, description = "Entity key")
    ),
    responses(
        (status = 200, description = "Entity revision list", body = project_entity_revision::EntityVersionsListResponse),
        (status = 400, description = "Invalid projId or domain")
    )
)]
pub(crate) async fn list_project_entity_versions(
    State(state): State<AppState>,
    AxumPath((proj_id, domain, entity_key)): AxumPath<(
        i64,
        project_entity_revision::ProjectEntityDomain,
        String,
    )>,
) -> Result<Json<project_entity_revision::EntityVersionsListResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    project_entity_revision::list_entity_versions(
        &state.session_db,
        proj_id,
        domain.as_str(),
        &entity_key,
    )
    .await
    .map(Json)
    .map_err(entity_revision_err)
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct EntityCompareQuery {
    from: String,
    to: String,
}

#[utoipa::path(
    get,
    path = "/v1/project/config/{proj_id}/entities/{domain}/{entity_key}/versions/compare",
    tag = "ProjectConfig",
    operation_id = "compare_project_entity_versions",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("domain" = project_entity_revision::ProjectEntityDomain, Path, description = "Entity domain"),
        ("entity_key" = String, Path, description = "Entity key"),
        ("from" = String, Query, description = "Source entity revision id"),
        ("to" = String, Query, description = "Target entity revision id")
    ),
    responses(
        (status = 200, description = "Entity revision diff", body = project_entity_revision::EntityCompareResponse),
        (status = 400, description = "Invalid projId or domain"),
        (status = 404, description = "Revision not found")
    )
)]
pub(crate) async fn compare_project_entity_versions(
    State(state): State<AppState>,
    AxumPath((proj_id, domain, entity_key)): AxumPath<(
        i64,
        project_entity_revision::ProjectEntityDomain,
        String,
    )>,
    Query(q): Query<EntityCompareQuery>,
) -> Result<Json<project_entity_revision::EntityCompareResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    project_entity_revision::compare_entity_versions(
        &state.session_db,
        proj_id,
        domain.as_str(),
        &entity_key,
        &q.from,
        &q.to,
    )
    .await
    .map(Json)
    .map_err(entity_revision_err)
}

#[utoipa::path(
    post,
    path = "/v1/project/config/{proj_id}/entities/{domain}/{entity_key}/restore",
    tag = "ProjectConfig",
    operation_id = "restore_project_entity_revision",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("domain" = project_entity_revision::ProjectEntityDomain, Path, description = "Entity domain"),
        ("entity_key" = String, Path, description = "Entity key")
    ),
    request_body = project_entity_revision::RestoreEntityRevisionRequest,
    responses(
        (status = 200, description = "Entity revision restored to draft", body = project_entity_revision::RestoreEntityRevisionResponse),
        (status = 400, description = "Invalid projId or domain"),
        (status = 404, description = "Revision or project not found")
    )
)]
pub(crate) async fn restore_project_entity_revision(
    State(state): State<AppState>,
    AxumPath((proj_id, domain, entity_key)): AxumPath<(
        i64,
        project_entity_revision::ProjectEntityDomain,
        String,
    )>,
    Json(req): Json<project_entity_revision::RestoreEntityRevisionRequest>,
) -> Result<Json<project_entity_revision::RestoreEntityRevisionResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    project_entity_revision::restore_entity_revision_to_draft(
        &state.session_db,
        proj_id,
        domain.as_str(),
        &entity_key,
        &req.entity_rev,
    )
    .await
    .map(Json)
    .map_err(entity_revision_err)
}

pub(crate) fn default_project_config_row(proj_id: i64) -> session_db::ProjectConfigRow {
    session_db::ProjectConfigRow {
        proj_id,
        content_rev: String::new(),
        stable_content_rev: None,
        draft_open: false,
        updated_at_ms: 0,
        rules_json: json!([]),
        mcp_servers_json: json!({}),
        skills_sources_json: json!([]),
        skills_json: json!([]),
        allowed_tools_json: json!([]),
        claude_md: None,
        git_sync_json: json!({}),
        solve_preflight_json: json!({"kind": "none"}),
        solve_orchestration_json: json!({"kind": "single_turn"}),
        language_pipeline_json: json!({}),
        extra_session_fields_json: json!([]),
        prompt_limits_json: project_config_apply::default_prompt_limits_json(),
        worker_profile_json: pool::default_worker_profile_json(),
        project_code: String::new(),
        project_description: String::new(),
    max_iterations: None,
    }
}

pub(crate) fn revision_row_from_upsert<'a>(
    proj_id: i64,
    content_rev: &'a str,
    created_at_ms: i64,
    upsert: &session_db::ProjectConfigUpsert<'a>,
) -> session_db::ProjectConfigRevisionRow {
    session_db::ProjectConfigRevisionRow {
        proj_id,
        content_rev: content_rev.to_string(),
        created_at_ms,
        note: None,
        rules_json: upsert.rules_json.clone(),
        mcp_servers_json: upsert.mcp_servers_json.clone(),
        skills_sources_json: upsert.skills_sources_json.clone(),
        skills_json: upsert.skills_json.clone(),
        allowed_tools_json: upsert.allowed_tools_json.clone(),
        claude_md: upsert.claude_md.map(str::to_string),
    }
}

pub(crate) fn project_config_version_entry_from_summary(
    r: &session_db::ProjectConfigRevisionSummary,
    effective: &str,
) -> ProjectConfigVersionEntry {
    ProjectConfigVersionEntry {
        content_rev: r.content_rev.clone(),
        created_at_ms: r.created_at_ms,
        is_draft: false,
        note: r.note.clone(),
        is_active: r.content_rev == effective,
        claude_in_db: r.claude_in_db,
        skills_count_db: r.skills_count_db,
        rules_count_db: r.rules_count_db,
        mcp_servers_count_db: r.mcp_servers_count_db,
    }
}

pub(crate) fn project_config_version_entry_from_draft(
    row: &session_db::ProjectConfigRow,
) -> ProjectConfigVersionEntry {
    let claude_in_db = row
        .claude_md
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());
    let skills_count_db = row.skills_json.as_array().map_or(0, |a| a.len() as i64);
    let rules_count_db = row.rules_json.as_array().map_or(0, |a| a.len() as i64);
    let mcp_servers_count_db = row
        .mcp_servers_json
        .as_object()
        .map_or(0, |o| o.len() as i64);
    ProjectConfigVersionEntry {
        content_rev: project_config_draft::DRAFT_CONTENT_REV.to_string(),
        created_at_ms: row.updated_at_ms,
        is_draft: true,
        note: None,
        is_active: false,
        claude_in_db,
        skills_count_db,
        rules_count_db,
        mcp_servers_count_db,
    }
}

pub(crate) async fn load_revision_for_compare(
    state: &AppState,
    proj_id: i64,
    content_rev: &str,
    active: &session_db::ProjectConfigRow,
) -> Result<session_db::ProjectConfigRevisionRow, ApiError> {
    if project_config_draft::is_draft_content_rev(content_rev) {
        if !active.draft_open {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("no open draft for proj {proj_id}"),
            ));
        }
        return Ok(project_config_draft::revision_row_from_config_row(
            active,
            project_config_draft::DRAFT_CONTENT_REV,
            None,
        ));
    }
    project_config_draft::require_formal_revision(&state.session_db, proj_id, content_rev)
        .await
        .map_err(draft_err)
}

pub(crate) fn revision_row_from_active(
    row: &session_db::ProjectConfigRow,
) -> session_db::ProjectConfigRevisionRow {
    session_db::ProjectConfigRevisionRow {
        proj_id: row.proj_id,
        content_rev: row.content_rev.clone(),
        created_at_ms: row.updated_at_ms,
        note: None,
        rules_json: row.rules_json.clone(),
        mcp_servers_json: row.mcp_servers_json.clone(),
        skills_sources_json: row.skills_sources_json.clone(),
        skills_json: row.skills_json.clone(),
        allowed_tools_json: row.allowed_tools_json.clone(),
        claude_md: row.claude_md.clone(),
    }
}

pub(crate) async fn archive_project_config_revision(
    state: &AppState,
    rev: session_db::ProjectConfigRevisionRow,
) -> Result<(), ApiError> {
    if project_config_draft::is_draft_content_rev(&rev.content_rev) {
        return Ok(());
    }
    let inserted = state
        .session_db
        .insert_project_config_revision_immutable(&rev)
        .await
        .map_err(|e| session_db_err(&e))?;
    if !inserted {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "revision {} already exists and cannot be changed",
                rev.content_rev
            ),
        ));
    }
    Ok(())
}

pub(crate) async fn activate_project_config_revision_row(
    state: &AppState,
    proj_id: i64,
    rev: session_db::ProjectConfigRevisionRow,
    sidecars: project_config_draft::ProjectConfigSidecars,
) -> Result<bool, ApiError> {
    let now = now_ms();
    state
        .session_db
        .upsert_project_config(session_db::ProjectConfigUpsert {
            proj_id,
            content_rev: &rev.content_rev,
            stable_content_rev: Some(rev.content_rev.as_str()),
            draft_open: false,
            updated_at_ms: now,
            rules_json: &rev.rules_json,
            mcp_servers_json: &rev.mcp_servers_json,
            skills_sources_json: &rev.skills_sources_json,
            skills_json: &rev.skills_json,
            allowed_tools_json: &rev.allowed_tools_json,
            claude_md: rev.claude_md.as_deref(),
            git_sync_json: &sidecars.git_sync_json,
            solve_preflight_json: &sidecars.solve_preflight_json,
            solve_orchestration_json: &sidecars.solve_orchestration_json,
            language_pipeline_json: &sidecars.language_pipeline_json,
            extra_session_fields_json: &sidecars.extra_session_fields_json,
            prompt_limits_json: &sidecars.prompt_limits_json,
            worker_profile_json: &sidecars.worker_profile_json,
            project_code: &sidecars.project_code,
            project_description: &sidecars.project_description,
        max_iterations: sidecars.max_iterations,
        })
        .await
        .map_err(|e| session_db_err(&e))?;
    let lock = get_proj_lock(state, proj_id).await;
    let _guard = lock.lock().await;
    // e2b worker reads project config from NAS `{cluster}/proj_N/home` (mounted ro as `/claw_ds`):
    // write the effective config there via nas-api on activate (the real bug fix). The host
    // `work_root/proj_N` materialization is kept for now (health / project-list rev sync).
    // Author: kejiqing
    apply_project_config_for_proj_inner(state, proj_id, true).await?;
    state
        .pool_clients
        .nas_layout()
        .materialize_proj_workspace(&state.session_db, proj_id)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("materialize project config to NAS failed: {e}"),
            )
        })?;
    let applied = project_config_apply::read_applied_content_rev(&proj_work_dir(
        &state.cfg.work_root,
        proj_id,
    ))
    .await;
    Ok(applied.as_deref() == Some(rev.content_rev.as_str()))
}

pub(crate) fn merge_git_sync_from_put(incoming: &Value, existing: &Value) -> Value {
    let mut inc = parse_git_sync_json(incoming);
    let ex = parse_git_sync_json(existing);
    let pat_id_in_incoming = incoming.get("gitPatId").is_some();
    if !pat_id_in_incoming {
        inc.git_pat_id = ex.git_pat_id;
    } else if incoming
        .get("gitPatId")
        .is_some_and(serde_json::Value::is_null)
    {
        inc.git_pat_id = None;
    }
    let uses_global_pat = inc
        .git_pat_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    if uses_global_pat {
        inc.git_token = None;
    } else if inc
        .git_token
        .as_deref()
        .map(str::trim)
        .as_ref()
        .is_none_or(|s| s.is_empty())
    {
        inc.git_token = ex.git_token;
    }
    if incoming.get("lastPullAtMs").is_none() {
        inc.last_pull_at_ms = ex.last_pull_at_ms;
        inc.last_pull_commit_id = ex.last_pull_commit_id;
        inc.last_pull_error = ex.last_pull_error;
    }
    git_sync_to_json(&inc)
}

pub(crate) async fn git_sync_json_for_api(state: &AppState, v: &Value) -> Value {
    let sync = parse_git_sync_json(v);
    let tokens = gateway_global_settings::load_git_pat_tokens(&state.session_db)
        .await
        .ok();
    let token_set = git_sync_token_set(&sync, tokens.as_ref());
    let mut j = git_sync_to_json(&sync);
    if let Some(obj) = j.as_object_mut() {
        obj.insert("gitTokenSet".into(), json!(token_set));
    }
    j
}

pub(crate) fn git_sync_token_set(
    sync: &project_git_sync::ProjectGitSync,
    tokens: Option<&gateway_global_settings::GitPatTokensStore>,
) -> bool {
    if sync
        .git_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    let Some(id) = sync
        .git_pat_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    tokens.is_some_and(|t| t.tokens.contains_key(id))
}

pub(crate) async fn load_project_config_or_default(
    state: &AppState,
    proj_id: i64,
) -> Result<session_db::ProjectConfigRow, ApiError> {
    Ok(state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .unwrap_or_else(|| default_project_config_row(proj_id)))
}

pub(crate) fn merge_skill_into_skills_json(skills_json: &mut Value, skill_name: &str, skill_content: &str) {
    if !skills_json.is_array() {
        *skills_json = json!([]);
    }
    let arr = skills_json.as_array_mut().expect("skills_json is array");
    for item in arr.iter_mut() {
        if item.get("skillName").and_then(Value::as_str) == Some(skill_name) {
            if let Some(obj) = item.as_object_mut() {
                obj.insert("skillContent".into(), json!(skill_content));
            }
            return;
        }
    }
    arr.push(json!({
        "skillName": skill_name,
        "skillContent": skill_content,
    }));
}

pub(crate) fn validate_skills_json(skills: &Value) -> Result<(), ApiError> {
    let arr = skills
        .as_array()
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "skillsJson must be a JSON array"))?;
    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("skillsJson[{i}] must be a JSON object"),
            )
        })?;
        let name = obj
            .get("skillName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("skillsJson[{i}] missing skillName"),
                )
            })?;
        validate_skill_name(name)?;
        if !obj.contains_key("skillContent") {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("skillsJson[{i}] missing skillContent"),
            ));
        }
    }
    Ok(())
}

pub(crate) fn reject_deprecated_skills_sources(sources: &Value) -> Result<(), ApiError> {
    if sources.as_array().is_some_and(|a| !a.is_empty()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "skillsSourcesJson is deprecated; use skillsJson (inline skills stored in project_config)",
        ));
    }
    Ok(())
}

pub(crate) async fn project_config_row_to_response(
    state: &AppState,
    row: session_db::ProjectConfigRow,
) -> ProjectConfigResponse {
    ProjectConfigResponse {
        proj_id: row.proj_id,
        content_rev: row.content_rev.clone(),
        stable_content_rev: row.stable_content_rev.clone(),
        draft_open: row.draft_open,
        updated_at_ms: row.updated_at_ms,
        rules_json: row.rules_json,
        mcp_servers_json: row.mcp_servers_json,
        skills_sources_json: row.skills_sources_json,
        skills_json: row.skills_json,
        allowed_tools_json: row.allowed_tools_json,
        claude_md: row.claude_md,
        git_sync_json: git_sync_json_for_api(state, &row.git_sync_json).await,
        solve_preflight_json: row.solve_preflight_json.clone(),
        solve_orchestration_json:
            gateway_solve_turn::project_orchestration::materialize_solve_orchestration_json(
                &row.solve_orchestration_json,
            ),
        language_pipeline_json:
            gateway_solve_turn::project_language_pipeline::materialize_language_pipeline_json(
                &row.language_pipeline_json,
            ),
        extra_session_fields_json: row.extra_session_fields_json,
        prompt_limits_json: row.prompt_limits_json,
        worker_profile_json: row.worker_profile_json,
        project_code: row.project_code,
        project_description: row.project_description,
        max_iterations: row.max_iterations,
    }
}

pub(crate) const SKILLS_SOURCES_FORBIDDEN_CRED_KEYS: &[&str] = &[
    "token",
    "gitToken",
    "accessToken",
    "password",
    "secret",
    "pat",
];

/// Git credentials for `project_config` skills sources: env only (`tokenEnv`), never in JSON/DB.
fn validate_skills_sources_json(sources: &Value) -> Result<(), ApiError> {
    let arr = sources.as_array().ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "skillsSourcesJson must be a JSON array",
        )
    })?;
    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("skillsSourcesJson[{i}] must be a JSON object"),
            )
        })?;
        for key in SKILLS_SOURCES_FORBIDDEN_CRED_KEYS {
            if obj.contains_key(*key) {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "skillsSourcesJson[{i}]: git credentials must not be stored in project_config; use tokenEnv pointing to a gateway environment variable"
                    ),
                ));
            }
        }
        let Some(git_url) = obj
            .get("gitUrl")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let is_http = git_url.starts_with("https://") || git_url.starts_with("http://");
        if is_http && git_url.contains('@') {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "skillsSourcesJson[{i}]: gitUrl must not embed userinfo; set tokenEnv to an env var name (git token is env-only)"
                ),
            ));
        }
        if is_http {
            let token_env = obj
                .get("tokenEnv")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(token_env) = token_env else {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "skillsSourcesJson[{i}]: tokenEnv is required for HTTP(S) gitUrl without embedded credentials"
                    ),
                ));
            };
            if !token_env
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "skillsSourcesJson[{i}]: tokenEnv must be an ASCII env var name [A-Za-z0-9_]"
                    ),
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_project_config_payload(req: &UpsertProjectConfigRequest) -> Result<(), ApiError> {
    if !req.rules_json.is_array() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "rulesJson must be a JSON array",
        ));
    }
    if !req.mcp_servers_json.is_object() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "mcpServersJson must be a JSON object",
        ));
    }
    if !req.allowed_tools_json.is_array() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "allowedToolsJson must be a JSON array",
        ));
    }
    reject_deprecated_skills_sources(&req.skills_sources_json)?;
    validate_skills_json(&req.skills_json)?;
    project_tools::validate_project_allowed_tools_json(&req.allowed_tools_json)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if let Some(ref sp) = req.solve_preflight_json {
        gateway_solve_turn::project_preflight::validate_solve_preflight_json(sp)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    }
    if let Some(ref so) = req.solve_orchestration_json {
        gateway_solve_turn::project_orchestration::validate_solve_orchestration_json(so)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    }
    if let Some(ref lp) = req.language_pipeline_json {
        gateway_solve_turn::project_language_pipeline::validate_language_pipeline_json(lp)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    }
    if let Some(ref esf) = req.extra_session_fields_json {
        project_extra_session::validate_project_extra_session_fields_json(esf)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    }
    if let Some(ref pl) = req.prompt_limits_json {
        project_config_apply::validate_prompt_limits_json(pl)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    }
    if let Some(ref wi) = req.worker_profile_json {
        pool::validate_worker_profile_json(wi)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    }
    if let Some(Some(0)) = req.max_iterations {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "maxIterations must be >= 1",
        ));
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/v1/project/config/{proj_id}",
    tag = "ProjectConfig",
    operation_id = "get_project_config",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Project configuration", body = ProjectConfigResponse),
        (status = 400, description = "Invalid projId"),
        (status = 404, description = "No project_config row")
    )
)]
pub(crate) async fn get_project_config(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<ProjectConfigResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let row = project_config_draft::row_for_editing(&state.session_db, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(row) = row else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for proj {proj_id}"),
        ));
    };
    Ok(Json(project_config_row_to_response(&state, row).await))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct PutProjectConfigResponse {
    #[serde(rename = "draftOpen")]
    draft_open: bool,
    #[serde(rename = "stableContentRev", skip_serializing_if = "Option::is_none")]
    stable_content_rev: Option<String>,
    #[serde(rename = "activeConfig")]
    active_config: ProjectConfigResponse,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct CommitProjectConfigDraftResponse {
    #[serde(rename = "savedContentRev")]
    saved_content_rev: String,
    activated: bool,
    #[serde(rename = "stableContentRev")]
    stable_content_rev: String,
    materialized: bool,
    #[serde(rename = "activeConfig")]
    active_config: ProjectConfigResponse,
}

#[utoipa::path(
    get,
    path = "/v1/project/config/{proj_id}/versions",
    tag = "ProjectConfig",
    operation_id = "list_project_config_versions",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Configuration version list", body = ProjectConfigVersionsResponse),
        (status = 400, description = "Invalid projId"),
        (status = 404, description = "No project_config row")
    )
)]
pub(crate) async fn list_project_config_versions(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<ProjectConfigVersionsResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let active = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(active) = active else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for proj {proj_id}"),
        ));
    };
    let revisions = state
        .session_db
        .list_project_config_revisions(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    let applied_content_rev = project_config_apply::read_applied_content_rev(&work_dir).await;
    let effective = project_config_draft::effective_formal_rev(&active)
        .map_err(draft_err)?
        .to_string();
    project_config_draft::ensure_formal_revision_recorded(
        &state.session_db,
        proj_id,
        &effective,
        &active,
    )
    .await
    .map_err(draft_err)?;
    let mut versions: Vec<ProjectConfigVersionEntry> = revisions
        .into_iter()
        .filter(|r| !project_config_draft::is_draft_content_rev(&r.content_rev))
        .map(|r| project_config_version_entry_from_summary(&r, &effective))
        .collect();
    if active.draft_open {
        versions.insert(0, project_config_version_entry_from_draft(&active));
    }
    Ok(Json(ProjectConfigVersionsResponse {
        proj_id,
        active_content_rev: effective,
        applied_content_rev,
        draft_open: active.draft_open,
        versions,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/project/config/{proj_id}/versions/compare",
    tag = "ProjectConfig",
    operation_id = "compare_project_config_versions",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("from" = String, Query, description = "Source content revision id"),
        ("to" = String, Query, description = "Target content revision id")
    ),
    responses(
        (status = 200, description = "Configuration revision diff", body = project_config_version::ProjectConfigCompareResponse),
        (status = 400, description = "Invalid projId or missing from/to"),
        (status = 404, description = "Project or revision not found")
    )
)]
pub(crate) async fn compare_project_config_versions(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Query(query): Query<CompareProjectConfigQuery>,
) -> Result<Json<project_config_version::ProjectConfigCompareResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let from = query.from.trim();
    let to = query.to.trim();
    if from.is_empty() || to.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "query params from and to are required",
        ));
    }
    let active = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(active) = active else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for proj {proj_id}"),
        ));
    };
    let from_row = load_revision_for_compare(&state, proj_id, from, &active).await?;
    let to_row = load_revision_for_compare(&state, proj_id, to, &active).await?;
    Ok(Json(project_config_version::compare_revision_rows(
        proj_id,
        project_config_draft::effective_formal_rev(&active).map_err(draft_err)?,
        &from_row,
        &to_row,
    )))
}

#[utoipa::path(
    post,
    path = "/v1/project/config/{proj_id}/versions/{content_rev}/activate",
    tag = "ProjectConfig",
    operation_id = "activate_project_config_version",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("content_rev" = String, Path, description = "Formal content revision id")
    ),
    responses(
        (status = 200, description = "Version activated and materialized", body = ActivateProjectConfigVersionResponse),
        (status = 400, description = "Invalid projId or draft contentRev"),
        (status = 404, description = "Project or revision not found")
    )
)]
pub(crate) async fn activate_project_config_version(
    State(state): State<AppState>,
    AxumPath((proj_id, content_rev)): AxumPath<(i64, String)>,
) -> Result<Json<ActivateProjectConfigVersionResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let content_rev = content_rev.trim();
    if content_rev.is_empty() || project_config_draft::is_draft_content_rev(content_rev) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "contentRev must be a saved (non-draft) version id",
        ));
    }
    let active_row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(active_row) = active_row else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for proj {proj_id}"),
        ));
    };
    let rev =
        project_config_draft::require_formal_revision(&state.session_db, proj_id, content_rev)
            .await
            .map_err(draft_err)?;
    let materialized = activate_project_config_revision_row(
        &state,
        proj_id,
        rev,
        project_config_draft::ProjectConfigSidecars::from_row(&active_row),
    )
    .await?;
    Ok(Json(ActivateProjectConfigVersionResponse {
        proj_id,
        active_content_rev: content_rev.to_string(),
        activated: true,
        materialized,
    }))
}

#[utoipa::path(
    put,
    path = "/v1/project/config/{proj_id}",
    tag = "ProjectConfig",
    operation_id = "put_project_config",
    params(("proj_id" = i64, Path, description = "Project ID")),
    request_body = UpsertProjectConfigRequest,
    responses(
        (status = 200, description = "Draft updated", body = PutProjectConfigResponse),
        (status = 400, description = "Invalid payload"),
        (status = 404, description = "No project_config row")
    )
)]
pub(crate) async fn put_project_config(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Json(req): Json<UpsertProjectConfigRequest>,
) -> Result<Json<PutProjectConfigResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let existing = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(existing) = existing else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for proj {proj_id}; create project first"),
        ));
    };
    let existing_git = existing.git_sync_json.clone();
    let git_sync_json = match &req.git_sync_json {
        Some(incoming) => merge_git_sync_from_put(incoming, &existing_git),
        None => existing_git,
    };
    let solve_preflight_json = match &req.solve_preflight_json {
        Some(incoming) => incoming.clone(),
        None => existing.solve_preflight_json.clone(),
    };
    let solve_orchestration_json = match &req.solve_orchestration_json {
        Some(incoming) => incoming.clone(),
        None => existing.solve_orchestration_json.clone(),
    };
    let language_pipeline_json = match &req.language_pipeline_json {
        Some(incoming) => incoming.clone(),
        None => existing.language_pipeline_json.clone(),
    };
    let extra_session_fields_json = match &req.extra_session_fields_json {
        Some(incoming) => incoming.clone(),
        None => existing.extra_session_fields_json.clone(),
    };
    let prompt_limits_json = match &req.prompt_limits_json {
        Some(incoming) => incoming.clone(),
        None => existing.prompt_limits_json.clone(),
    };
    let worker_profile_json = match &req.worker_profile_json {
        Some(incoming) => incoming.clone(),
        None => existing.worker_profile_json.clone(),
    };
    let max_iterations = crate::max_iterations::parse_project_max_iterations_put(
        req.max_iterations,
        existing.max_iterations,
    )
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    let req_for_validate = UpsertProjectConfigRequest {
        content_rev: String::new(),
        rules_json: req.rules_json.clone(),
        mcp_servers_json: req.mcp_servers_json.clone(),
        skills_sources_json: req.skills_sources_json.clone(),
        skills_json: req.skills_json.clone(),
        allowed_tools_json: req.allowed_tools_json.clone(),
        claude_md: req.claude_md.clone(),
        git_sync_json: Some(git_sync_json.clone()),
        solve_preflight_json: Some(solve_preflight_json.clone()),
        solve_orchestration_json: Some(solve_orchestration_json.clone()),
        language_pipeline_json: Some(language_pipeline_json.clone()),
        extra_session_fields_json: Some(extra_session_fields_json.clone()),
        prompt_limits_json: Some(prompt_limits_json.clone()),
        worker_profile_json: Some(worker_profile_json.clone()),
        max_iterations: Some(max_iterations),
    };
    validate_project_config_payload(&req_for_validate)?;
    preflight_plugin_api::validate_solve_preflight_plugin_refs(
        &state.session_db,
        &solve_preflight_json,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    gateway_global_settings::validate_git_sync_json_with_global(&state.session_db, &git_sync_json)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    project_config_draft::ensure_draft(&state.session_db, proj_id)
        .await
        .map_err(draft_err)?;
    let effective = project_config_draft::effective_formal_rev(&existing)
        .map_err(draft_err)?
        .to_string();
    let now = now_ms();
    let upsert = session_db::ProjectConfigUpsert {
        proj_id,
        content_rev: project_config_draft::DRAFT_CONTENT_REV,
        stable_content_rev: Some(effective.as_str()),
        draft_open: true,
        updated_at_ms: now,
        rules_json: &req.rules_json,
        mcp_servers_json: &req.mcp_servers_json,
        skills_sources_json: &req.skills_sources_json,
        skills_json: &req.skills_json,
        allowed_tools_json: &req.allowed_tools_json,
        claude_md: req.claude_md.as_deref(),
        git_sync_json: &git_sync_json,
        solve_preflight_json: &solve_preflight_json,
        solve_orchestration_json: &solve_orchestration_json,
        language_pipeline_json: &language_pipeline_json,
        extra_session_fields_json: &extra_session_fields_json,
        prompt_limits_json: &prompt_limits_json,
        worker_profile_json: &worker_profile_json,
        project_code: &existing.project_code,
        project_description: &existing.project_description,
        max_iterations,
    };
    state
        .session_db
        .upsert_project_config(upsert)
        .await
        .map_err(|e| session_db_err(&e))?;
    if req.worker_profile_json.is_some() {
        let pool = state.pool_clients.clone();
        tokio::spawn(async move {
            if let Err(e) = pool.reconcile_project_worker(proj_id).await {
                tracing::warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    error = %e,
                    "post worker_profile reconcile failed (best-effort)"
                );
            }
        });
    }
    project_entity_revision::record_draft_put_sidecars(
        &state.session_db,
        proj_id,
        &existing,
        &req.rules_json,
        &req.skills_json,
        &req.mcp_servers_json,
        req.claude_md.as_deref(),
        &req.allowed_tools_json,
        now,
    )
    .await
    .map_err(entity_revision_err)?;
    let active = project_config_draft::row_for_editing(&state.session_db, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists after upsert");
    Ok(Json(PutProjectConfigResponse {
        draft_open: true,
        stable_content_rev: active.stable_content_rev.clone(),
        active_config: project_config_row_to_response(&state, active).await,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/project/config/{proj_id}/versions/commit",
    tag = "ProjectConfig",
    operation_id = "commit_project_config_draft",
    params(("proj_id" = i64, Path, description = "Project ID")),
    request_body = CommitProjectConfigDraftRequest,
    responses(
        (status = 200, description = "Draft committed as new formal revision", body = CommitProjectConfigDraftResponse),
        (status = 400, description = "Invalid projId or no open draft"),
        (status = 404, description = "No project_config row")
    )
)]
pub(crate) async fn commit_project_config_draft(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Json(req): Json<CommitProjectConfigDraftRequest>,
) -> Result<Json<CommitProjectConfigDraftResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let note = project_config_draft::normalize_revision_note(req.note);
    let row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("no project_config for proj {proj_id}"),
            )
        })?;
    if !row.draft_open {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "no open draft to commit; edit config first",
        ));
    }
    let git_sync_json = row.git_sync_json.clone();
    let prev_stable = project_config_draft::effective_formal_rev(&row)
        .map_err(draft_err)?
        .to_string();
    project_config_draft::ensure_formal_revision_recorded(
        &state.session_db,
        proj_id,
        &prev_stable,
        &row,
    )
    .await
    .map_err(draft_err)?;
    let now = now_ms();
    let saved = project_config_draft::allocate_formal_content_rev(&state.session_db, proj_id, now)
        .await
        .map_err(draft_err)?;
    let rev = project_config_draft::revision_row_from_config_row(&row, &saved, note);
    archive_project_config_revision(&state, rev).await?;
    let active = project_config_draft::close_draft_to_stable(
        &state.session_db,
        proj_id,
        &prev_stable,
        project_config_draft::ProjectConfigSidecars {
            git_sync_json,
            solve_preflight_json: row.solve_preflight_json.clone(),
            solve_orchestration_json: row.solve_orchestration_json.clone(),
            language_pipeline_json: row.language_pipeline_json.clone(),
            extra_session_fields_json: row.extra_session_fields_json.clone(),
            prompt_limits_json: row.prompt_limits_json.clone(),
            worker_profile_json: row.worker_profile_json.clone(),
            project_code: row.project_code.clone(),
            project_description: row.project_description.clone(),
            max_iterations: row.max_iterations,
        },
    )
    .await
    .map_err(draft_err)?;
    Ok(Json(CommitProjectConfigDraftResponse {
        saved_content_rev: saved,
        activated: false,
        stable_content_rev: prev_stable,
        materialized: false,
        active_config: project_config_row_to_response(&state, active).await,
    }))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct PatchProjectConfigVersionNoteRequest {
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct PatchProjectConfigVersionNoteResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "contentRev")]
    content_rev: String,
    #[serde(rename = "note", skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    saved: bool,
}

#[utoipa::path(
    patch,
    path = "/v1/project/config/{proj_id}/versions/{content_rev}",
    tag = "ProjectConfig",
    operation_id = "patch_project_config_version_note",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("content_rev" = String, Path, description = "Formal content revision id")
    ),
    request_body = PatchProjectConfigVersionNoteRequest,
    responses(
        (status = 200, description = "Revision note updated", body = PatchProjectConfigVersionNoteResponse),
        (status = 400, description = "Invalid projId or draft revision"),
        (status = 404, description = "Revision not found")
    )
)]
pub(crate) async fn patch_project_config_version_note(
    State(state): State<AppState>,
    AxumPath((proj_id, content_rev)): AxumPath<(i64, String)>,
    Json(req): Json<PatchProjectConfigVersionNoteRequest>,
) -> Result<Json<PatchProjectConfigVersionNoteResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let content_rev = content_rev.trim();
    if content_rev.is_empty() || project_config_draft::is_draft_content_rev(content_rev) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cannot set note on draft revision",
        ));
    }
    project_config_draft::require_formal_revision(&state.session_db, proj_id, content_rev)
        .await
        .map_err(draft_err)?;
    let note = project_config_draft::normalize_revision_note(req.note);
    let saved = state
        .session_db
        .update_project_config_revision_note(proj_id, content_rev, note.as_deref())
        .await
        .map_err(|e| session_db_err(&e))?;
    if !saved {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no revision {content_rev} for proj {proj_id}"),
        ));
    }
    Ok(Json(PatchProjectConfigVersionNoteResponse {
        proj_id,
        content_rev: content_rev.to_string(),
        note,
        saved: true,
    }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct DeleteProjectConfigVersionResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "contentRev")]
    content_rev: String,
    deleted: bool,
}

#[utoipa::path(
    delete,
    path = "/v1/project/config/{proj_id}/versions/{content_rev}",
    tag = "ProjectConfig",
    operation_id = "delete_project_config_version",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("content_rev" = String, Path, description = "Formal content revision id")
    ),
    responses(
        (status = 200, description = "Revision deleted", body = DeleteProjectConfigVersionResponse),
        (status = 400, description = "Invalid projId or draft revision"),
        (status = 404, description = "Revision not found"),
        (status = 409, description = "Cannot delete effective contentRev")
    )
)]
pub(crate) async fn delete_project_config_version(
    State(state): State<AppState>,
    AxumPath((proj_id, content_rev)): AxumPath<(i64, String)>,
) -> Result<Json<DeleteProjectConfigVersionResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let content_rev = content_rev.trim();
    if content_rev.is_empty() || project_config_draft::is_draft_content_rev(content_rev) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cannot delete draft revision",
        ));
    }
    let row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(row) = row else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for proj {proj_id}"),
        ));
    };
    let effective = project_config_draft::effective_formal_rev(&row).map_err(draft_err)?;
    if content_rev == effective {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "cannot delete the effective contentRev; activate another version first",
        ));
    }
    let deleted = state
        .session_db
        .delete_project_config_revision(proj_id, content_rev)
        .await
        .map_err(|e| session_db_err(&e))?;
    if !deleted {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no revision {content_rev} for proj {proj_id}"),
        ));
    }
    Ok(Json(DeleteProjectConfigVersionResponse {
        proj_id,
        content_rev: content_rev.to_string(),
        deleted: true,
    }))
}

#[cfg(test)]
mod max_iterations_project_response_tests {
    use super::*;

    #[test]
    fn project_response_serializes_max_iterations() {
        let response = ProjectConfigResponse {
            proj_id: 1,
            content_rev: "rev".into(),
            stable_content_rev: Some("rev".into()),
            draft_open: false,
            updated_at_ms: 0,
            rules_json: json!([]),
            mcp_servers_json: json!({}),
            skills_sources_json: json!([]),
            skills_json: json!([]),
            allowed_tools_json: json!([]),
            claude_md: None,
            git_sync_json: json!({}),
            solve_preflight_json: json!({"kind": "none"}),
            solve_orchestration_json: json!({"kind": "single_turn"}),
            language_pipeline_json: json!({}),
            extra_session_fields_json: json!([]),
            prompt_limits_json: json!({}),
            worker_profile_json: json!({"mode": "strict"}),
            project_code: String::new(),
            project_description: String::new(),
            max_iterations: Some(5),
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["maxIterations"], 5);
    }
}

