// Fragment of routes::app (include!). Author: kejiqing

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct UpdateProjectClaudeRequest {
    content: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct UpsertProjectSkillRequest {
    #[serde(rename = "skillName")]
    skill_name: String,
    #[serde(rename = "skillContent")]
    skill_content: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ProjectToolsCatalogResponse {
    tools: Vec<project_tools::ToolCatalogEntry>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ProjectClaudeResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    path: String,
    exists: bool,
    content: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ProjectSkillResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "skillName")]
    skill_name: String,
    #[serde(rename = "skillPath")]
    skill_path: String,
    created: bool,
    updated: bool,
    #[serde(rename = "bytesWritten")]
    bytes_written: usize,
    #[serde(rename = "workDir")]
    work_dir: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct EffectivePromptResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    sections: Vec<String>,
    message: String,
    /// `user` = project `claudeMd` override only; `system` = DB scaffold + project context.
    #[serde(rename = "promptSource")]
    prompt_source: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct DsSkillEntry {
    skill_name: String,
    skill_content: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct DsSkillsListResponse {
    proj_id: i64,
    skills: Vec<DsSkillEntry>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct DsSkillGetResponse {
    proj_id: i64,
    skill_name: String,
    skill_content: String,
}

pub(crate) async fn project_selected_allowed_tools(
    state: &AppState,
    proj_id: i64,
) -> Result<Option<Vec<String>>, ApiError> {
    let row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let selected = project_tools::parse_allowed_tools_json(&row.allowed_tools_json)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if selected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selected))
    }
}

#[utoipa::path(
    get,
    path = "/v1/project/tools/catalog",
    tag = "ProjectAssets",
    operation_id = "get_project_tools_catalog",
    responses(
        (status = 200, description = "Gateway-registered tool catalog", body = ProjectToolsCatalogResponse)
    )
)]
pub(crate) async fn get_project_tools_catalog(
    State(_state): State<AppState>,
) -> Json<ProjectToolsCatalogResponse> {
    Json(ProjectToolsCatalogResponse {
        tools: project_tools::gateway_registered_tool_catalog(),
    })
}

#[utoipa::path(
    get,
    path = "/v1/project/claude/{proj_id}",
    tag = "ProjectAssets",
    operation_id = "get_project_claude_md",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Project CLAUDE.md content", body = ProjectClaudeResponse),
        (status = 400, description = "Invalid projId")
    )
)]
pub(crate) async fn get_project_claude_md(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<ProjectClaudeResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let (home_claude_md_path, root_claude_md_path) = project_claude_paths(&work_dir);
    if let Some(row) = project_config_draft::row_for_editing(&state.session_db, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        let text = row.claude_md.unwrap_or_default();
        return Ok(Json(ProjectClaudeResponse {
            proj_id,
            work_dir: work_dir.display().to_string(),
            path: home_claude_md_path.display().to_string(),
            exists: !text.trim().is_empty(),
            content: text,
        }));
    }
    let content = fs::read_to_string(&home_claude_md_path).await;
    let (exists, content) = match content {
        Ok(text) => (true, text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::read_to_string(&root_claude_md_path).await {
                Ok(text) => (true, text),
                Err(root_err) if root_err.kind() == std::io::ErrorKind::NotFound => {
                    (false, String::new())
                }
                Err(root_err) => {
                    return Err(ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("read CLAUDE.md failed: {root_err}"),
                    ));
                }
            }
        }
        Err(error) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read CLAUDE.md failed: {error}"),
            ));
        }
    };
    Ok(Json(ProjectClaudeResponse {
        proj_id,
        work_dir: work_dir.display().to_string(),
        path: home_claude_md_path.display().to_string(),
        exists,
        content,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/project/claude/{proj_id}",
    tag = "ProjectAssets",
    operation_id = "update_project_claude_md",
    params(("proj_id" = i64, Path, description = "Project ID")),
    request_body = UpdateProjectClaudeRequest,
    responses(
        (status = 200, description = "CLAUDE.md updated in draft", body = ProjectClaudeResponse),
        (status = 400, description = "Invalid projId"),
        (status = 404, description = "No project_config row")
    )
)]
pub(crate) async fn update_project_claude_md(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Json(req): Json<UpdateProjectClaudeRequest>,
) -> Result<Json<ProjectClaudeResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let lock = get_proj_lock(&state, proj_id).await;
    let _guard = lock.lock().await;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let Some(_) = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for proj {proj_id}; create project first"),
        ));
    };
    project_config_draft::ensure_draft(&state.session_db, proj_id)
        .await
        .map_err(draft_err)?;
    let mut row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists");
    row.claude_md = Some(req.content.clone());
    row.draft_open = true;
    row.content_rev = project_config_draft::DRAFT_CONTENT_REV.to_string();
    row.updated_at_ms = now_ms();
    let saved = req.content.clone();
    state
        .session_db
        .upsert_project_config(project_config_draft::upsert_from_row(
            &row,
            project_config_draft::DRAFT_CONTENT_REV,
            row.updated_at_ms,
            row.claude_md.as_deref(),
            row.stable_content_rev.as_deref(),
        ))
        .await
        .map_err(|e| session_db_err(&e))?;
    let now = row.updated_at_ms;
    project_entity_revision::append_claude(&state.session_db, proj_id, &saved, now)
        .await
        .map_err(entity_revision_err)?;
    let claude_md_path = work_dir.join("home/CLAUDE.md");
    Ok(Json(ProjectClaudeResponse {
        proj_id,
        work_dir: work_dir.display().to_string(),
        path: claude_md_path.display().to_string(),
        exists: true,
        content: saved,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/project/skills/{proj_id}",
    tag = "ProjectAssets",
    operation_id = "upsert_project_skill",
    params(("proj_id" = i64, Path, description = "Project ID")),
    request_body = UpsertProjectSkillRequest,
    responses(
        (status = 200, description = "Skill upserted in draft", body = ProjectSkillResponse),
        (status = 400, description = "Invalid projId or skill name"),
        (status = 404, description = "No project_config row")
    )
)]
pub(crate) async fn upsert_project_skill(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Json(req): Json<UpsertProjectSkillRequest>,
) -> Result<Json<ProjectSkillResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let skill_name = req.skill_name.trim().to_string();
    validate_skill_name(&skill_name)?;
    let work_dir = proj_work_dir(&state.cfg.work_root, proj_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let skill_rel = PathBuf::from("home")
        .join("skills")
        .join(&skill_name)
        .join("SKILL.md");
    let skill_path = work_dir.join(&skill_rel);
    let lock = get_proj_lock(&state, proj_id).await;
    let _guard = lock.lock().await;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let Some(_) = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for proj {proj_id}; create project first"),
        ));
    };
    project_config_draft::ensure_draft(&state.session_db, proj_id)
        .await
        .map_err(draft_err)?;
    let mut row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists");
    let existed = row.skills_json.as_array().is_some_and(|a| {
        a.iter()
            .any(|item| item.get("skillName").and_then(Value::as_str) == Some(skill_name.as_str()))
    });
    merge_skill_into_skills_json(&mut row.skills_json, &skill_name, &req.skill_content);
    row.draft_open = true;
    row.content_rev = project_config_draft::DRAFT_CONTENT_REV.to_string();
    row.updated_at_ms = now_ms();
    let skill_body = row
        .skills_json
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|item| {
                    item.get("skillName").and_then(Value::as_str) == Some(skill_name.as_str())
                })
                .cloned()
        })
        .unwrap_or_else(|| {
            json!({
                "skillName": skill_name,
                "skillContent": req.skill_content,
            })
        });
    state
        .session_db
        .upsert_project_config(project_config_draft::upsert_from_row(
            &row,
            project_config_draft::DRAFT_CONTENT_REV,
            row.updated_at_ms,
            row.claude_md.as_deref(),
            row.stable_content_rev.as_deref(),
        ))
        .await
        .map_err(|e| session_db_err(&e))?;
    project_entity_revision::append_skill(
        &state.session_db,
        proj_id,
        &skill_name,
        skill_body,
        row.updated_at_ms,
    )
    .await
    .map_err(entity_revision_err)?;
    Ok(Json(ProjectSkillResponse {
        proj_id,
        skill_name,
        skill_path: skill_path.display().to_string(),
        created: !existed,
        updated: existed,
        bytes_written: req.skill_content.len(),
        work_dir: work_dir.display().to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/v1/project/prompt/{proj_id}/effective",
    tag = "ProjectAssets",
    operation_id = "get_effective_prompt",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Effective system prompt sections", body = EffectivePromptResponse),
        (status = 400, description = "Invalid projId")
    )
)]
pub(crate) async fn get_effective_prompt(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<EffectivePromptResponse>, ApiError> {
    build_effective_prompt_response(&state, proj_id, true)
        .await
        .map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/project/prompt/{proj_id}/effective",
    tag = "ProjectAssets",
    operation_id = "post_effective_prompt",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Effective system prompt after forced apply", body = EffectivePromptResponse),
        (status = 400, description = "Invalid projId")
    )
)]
pub(crate) async fn post_effective_prompt(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<EffectivePromptResponse>, ApiError> {
    build_effective_prompt_response(&state, proj_id, true)
        .await
        .map(Json)
}

pub(crate) async fn build_effective_prompt_response(
    state: &AppState,
    proj_id: i64,
    force_apply: bool,
) -> Result<EffectivePromptResponse, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let work_dir = state.cfg.work_root.join(format!("proj_{proj_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;

    let row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    apply_project_config_for_proj(state, proj_id, force_apply).await?;
    let model_family = gateway_global_settings::load_active_llm_config_public(&state.session_db)
        .await
        .ok()
        .flatten()
        .map(|active| active.model_name)
        .filter(|name| !name.trim().is_empty());
    let sections = load_system_prompt(
        work_dir.to_path_buf(),
        default_system_date(),
        std::env::consts::OS,
        "unknown",
        model_family,
        None,
    )
    .map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("load system prompt failed: {e}"),
        )
    })?;
    let message = sections.join("\n\n");
    let prompt_source = if row
        .as_ref()
        .and_then(|r| r.claude_md.as_deref())
        .is_some_and(|s| !s.trim().is_empty())
    {
        "user"
    } else {
        "system"
    }
    .to_string();
    Ok(EffectivePromptResponse {
        proj_id,
        work_dir: work_dir.display().to_string(),
        sections,
        message,
        prompt_source,
    })
}

pub(crate) fn is_safe_skill_dir_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

pub(crate) fn skills_from_skills_json(skills_json: &Value) -> Vec<DsSkillEntry> {
    let mut skills = Vec::new();
    let Some(arr) = skills_json.as_array() else {
        return skills;
    };
    for item in arr {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some(name) = obj.get("skillName").and_then(Value::as_str) else {
            continue;
        };
        let content = obj
            .get("skillContent")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        skills.push(DsSkillEntry {
            skill_name: name.to_string(),
            skill_content: content,
        });
    }
    skills
}

pub(crate) async fn load_skills_from_proj_workdir(work_dir: &Path) -> std::io::Result<Vec<DsSkillEntry>> {
    let skills_root = work_dir.join("home").join("skills");
    let mut out = Vec::new();
    if !fs::metadata(&skills_root).await.is_ok_and(|m| m.is_dir()) {
        return Ok(out);
    }
    let mut rd = fs::read_dir(&skills_root).await?;
    while let Some(entry) = rd.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let skill_name = entry.file_name().to_string_lossy().to_string();
        if !is_safe_skill_dir_name(&skill_name) {
            continue;
        }
        let path = entry.path().join("SKILL.md");
        if !fs::metadata(&path).await.is_ok_and(|m| m.is_file()) {
            continue;
        }
        let skill_content = fs::read_to_string(&path).await?;
        out.push(DsSkillEntry {
            skill_name,
            skill_content,
        });
    }
    out.sort_by(|a, b| a.skill_name.cmp(&b.skill_name));
    Ok(out)
}

#[utoipa::path(
    get,
    path = "/v1/skills/{proj_id}",
    tag = "ProjectAssets",
    operation_id = "list_proj_skills",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Skills list", body = DsSkillsListResponse),
        (status = 400, description = "Invalid projId")
    )
)]
pub(crate) async fn list_proj_skills(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<DsSkillsListResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let work_dir = state.cfg.work_root.join(format!("proj_{proj_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    if let Some(row) = project_config_draft::row_for_editing(&state.session_db, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        if row.draft_open {
            let mut skills = skills_from_skills_json(&row.skills_json);
            skills.sort_by(|a, b| a.skill_name.cmp(&b.skill_name));
            return Ok(Json(DsSkillsListResponse { proj_id, skills }));
        }
        if row.skills_json.as_array().is_some_and(|a| !a.is_empty()) {
            let mut skills = skills_from_skills_json(&row.skills_json);
            skills.sort_by(|a, b| a.skill_name.cmp(&b.skill_name));
            return Ok(Json(DsSkillsListResponse { proj_id, skills }));
        }
    }
    let skills = load_skills_from_proj_workdir(&work_dir)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("list skills failed: {e}"),
            )
        })?;
    Ok(Json(DsSkillsListResponse { proj_id, skills }))
}

#[utoipa::path(
    get,
    path = "/v1/skills/{proj_id}/{skill_name}",
    tag = "ProjectAssets",
    operation_id = "get_proj_skill",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("skill_name" = String, Path, description = "Skill directory name")
    ),
    responses(
        (status = 200, description = "Skill content", body = DsSkillGetResponse),
        (status = 400, description = "Invalid projId or skill name"),
        (status = 404, description = "Skill not found")
    )
)]
pub(crate) async fn get_proj_skill(
    State(state): State<AppState>,
    AxumPath((proj_id, skill_name)): AxumPath<(i64, String)>,
) -> Result<Json<DsSkillGetResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    if !is_safe_skill_dir_name(&skill_name) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid skill_name"));
    }
    let work_dir = state.cfg.work_root.join(format!("proj_{proj_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    if let Some(row) = project_config_draft::row_for_editing(&state.session_db, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        if row.draft_open {
            for entry in skills_from_skills_json(&row.skills_json) {
                if entry.skill_name == skill_name {
                    return Ok(Json(DsSkillGetResponse {
                        proj_id,
                        skill_name,
                        skill_content: entry.skill_content,
                    }));
                }
            }
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("skill not found in draft: {skill_name}"),
            ));
        }
        if row.skills_json.as_array().is_some_and(|a| !a.is_empty()) {
            for entry in skills_from_skills_json(&row.skills_json) {
                if entry.skill_name == skill_name {
                    return Ok(Json(DsSkillGetResponse {
                        proj_id,
                        skill_name,
                        skill_content: entry.skill_content,
                    }));
                }
            }
        }
    }
    let path = work_dir
        .join("home")
        .join("skills")
        .join(&skill_name)
        .join("SKILL.md");
    let skill_content = match fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("skill not found: {skill_name}"),
            ));
        }
        Err(e) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read skill failed: {e}"),
            ));
        }
    };
    Ok(Json(DsSkillGetResponse {
        proj_id,
        skill_name,
        skill_content,
    }))
}

