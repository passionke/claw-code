// Fragment of routes::app (include!). Master observer HTTP + MCP. Author: kejiqing

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct PutProjectRoleRequest {
    #[serde(rename = "projectRole")]
    project_role: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PutProjectRoleResponse {
    proj_id: i64,
    project_role: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct PutApprenticesRequest {
    #[serde(rename = "apprenticeProjIds")]
    apprentice_proj_ids: Vec<i64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApprenticesResponse {
    master_proj_id: i64,
    links: Vec<master_observer::ProjectMasterLinkRow>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct PutScheduleRequest {
    #[serde(rename = "jobId")]
    job_id: Option<String>,
    #[serde(rename = "scheduleKind", default = "default_daily_kind")]
    schedule_kind: String,
    #[serde(rename = "runAtHhmm", default = "default_hhmm")]
    run_at_hhmm: String,
    weekday: Option<i32>,
    #[serde(default = "default_true_sched")]
    enabled: bool,
    #[serde(rename = "promptTemplate")]
    prompt_template: Option<String>,
}

fn default_daily_kind() -> String {
    "daily".into()
}
fn default_hhmm() -> String {
    "02:00".into()
}
fn default_true_sched() -> bool {
    true
}

#[utoipa::path(
    put,
    path = "/v1/projects/{proj_id}/role",
    tag = "Master",
    operation_id = "put_master_role",
    params(("proj_id" = i64, Path, description = "Project id")),
    request_body = PutProjectRoleRequest,
    responses((status = 200, body = PutProjectRoleResponse), (status = 400, description = "bad role"))
)]
pub(crate) async fn put_master_role(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Json(req): Json<PutProjectRoleRequest>,
) -> Result<Json<PutProjectRoleResponse>, ApiError> {
    let role = master_observer::validate_project_role(&req.project_role)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if role == master_observer::PROJECT_ROLE_MASTER {
        master_observer::seed_master_project(&state.session_db, proj_id)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
        apply_project_config_for_proj(&state, proj_id, true).await?;
        let _ = state.pool_clients.reconcile_project_worker(proj_id).await;
    } else if role == master_observer::PROJECT_ROLE_NORMAL {
        state
            .session_db
            .set_project_role(proj_id, role)
            .await
            .map_err(|e| session_db_err(&e))?;
    } else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "observation role is assigned only via apprentice pairing",
        ));
    }
    Ok(Json(PutProjectRoleResponse {
        proj_id,
        project_role: role.to_string(),
    }))
}

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/apprentices",
    tag = "Master",
    operation_id = "get_master_apprentices",
    params(("proj_id" = i64, Path, description = "Master project id")),
    responses((status = 200, body = ApprenticesResponse), (status = 400, description = "not master"))
)]
pub(crate) async fn get_master_apprentices(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<ApprenticesResponse>, ApiError> {
    ensure_master_role(&state, proj_id).await?;
    let links = state
        .session_db
        .list_master_links(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(ApprenticesResponse {
        master_proj_id: proj_id,
        links,
    }))
}

#[utoipa::path(
    put,
    path = "/v1/projects/{proj_id}/apprentices",
    tag = "Master",
    operation_id = "put_master_apprentices",
    params(("proj_id" = i64, Path, description = "Master project id")),
    request_body = PutApprenticesRequest,
    responses((status = 200, body = ApprenticesResponse), (status = 400, description = "bad request"))
)]
pub(crate) async fn put_master_apprentices(
    State(state): State<AppState>,
    AxumPath(master_proj_id): AxumPath<i64>,
    Json(req): Json<PutApprenticesRequest>,
) -> Result<Json<ApprenticesResponse>, ApiError> {
    ensure_master_role(&state, master_proj_id).await?;
    let desired: std::collections::HashSet<i64> = req.apprentice_proj_ids.into_iter().collect();
    let existing = state
        .session_db
        .list_master_links(master_proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    for link in &existing {
        if !desired.contains(&link.apprentice_proj_id) && !link.orphaned {
            state
                .session_db
                .mark_master_link_orphaned(master_proj_id, link.apprentice_proj_id)
                .await
                .map_err(|e| session_db_err(&e))?;
        }
    }
    for aid in desired {
        if aid == master_proj_id {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "master cannot apprentice itself",
            ));
        }
        if let Some(link) = existing.iter().find(|l| l.apprentice_proj_id == aid) {
            if link.orphaned {
                let mut restored = link.clone();
                restored.orphaned = false;
                restored.updated_at_ms = now_ms();
                state
                    .session_db
                    .upsert_master_link(&restored)
                    .await
                    .map_err(|e| session_db_err(&e))?;
            }
            continue;
        }
        create_observation_for_apprentice(&state, master_proj_id, aid).await?;
    }
    let links = state
        .session_db
        .list_master_links(master_proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(ApprenticesResponse {
        master_proj_id,
        links,
    }))
}

async fn create_observation_for_apprentice(
    state: &AppState,
    master_proj_id: i64,
    apprentice_proj_id: i64,
) -> Result<(), ApiError> {
    let apprentice = state
        .session_db
        .get_project_config(apprentice_proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("apprentice {apprentice_proj_id} not found"),
            )
        })?;
    let a_role = state
        .session_db
        .get_project_role(apprentice_proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if a_role == master_observer::PROJECT_ROLE_MASTER
        || a_role == master_observer::PROJECT_ROLE_OBSERVATION
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("proj {apprentice_proj_id} cannot be an apprentice (role={a_role})"),
        ));
    }
    let source = project_config_draft::row_for_materialize(&state.session_db, apprentice_proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("apprentice {apprentice_proj_id} has no stable config"),
            )
        })?;

    let obs_id = resolve_create_proj_id(state, None).await?;
    let code = format!(
        "obs-m{}-a{}",
        master_proj_id, apprentice_proj_id
    );
    let desc = format!(
        "Observation space for apprentice {apprentice_proj_id} (master {master_proj_id})"
    );
    let work_dir = proj_work_dir(&state.cfg.work_root, obs_id);
    scaffold_proj_workspace(&work_dir, obs_id).await?;
    master_observer::clone_stable_config_onto_project(
        &state.session_db,
        &source,
        obs_id,
        &code,
        &desc,
        &master_observer::zero_pool_worker_profile_json(),
        None,
        None,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    state
        .session_db
        .set_project_role(obs_id, master_observer::PROJECT_ROLE_OBSERVATION)
        .await
        .map_err(|e| session_db_err(&e))?;
    apply_project_config_for_proj(state, obs_id, true).await?;
    let _ = state.pool_clients.reconcile_project_worker(obs_id).await;
    let now = now_ms();
    let link = master_observer::ProjectMasterLinkRow {
        master_proj_id,
        apprentice_proj_id,
        observation_proj_id: obs_id,
        orphaned: false,
        created_at_ms: now,
        updated_at_ms: now,
    };
    state
        .session_db
        .upsert_master_link(&link)
        .await
        .map_err(|e| session_db_err(&e))?;
    let _ = apprentice;
    Ok(())
}

async fn ensure_master_role(state: &AppState, proj_id: i64) -> Result<(), ApiError> {
    let role = state
        .session_db
        .get_project_role(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if role != master_observer::PROJECT_ROLE_MASTER {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("proj {proj_id} is not master"),
        ));
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/repair-runs",
    tag = "Master",
    operation_id = "list_master_repair_runs",
    params(("proj_id" = i64, Path, description = "Master project id")),
    responses((status = 200, description = "repair runs"), (status = 400, description = "not master"))
)]
pub(crate) async fn list_master_repair_runs(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<Value>, ApiError> {
    ensure_master_role(&state, proj_id).await?;
    let runs = state
        .session_db
        .list_repair_runs(proj_id, 50)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(json!({"runs": runs})))
}

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/repair-runs/{run_id}",
    tag = "Master",
    operation_id = "get_master_repair_run",
    params(
        ("proj_id" = i64, Path, description = "Master project id"),
        ("run_id" = String, Path, description = "Repair run id")
    ),
    responses((status = 200, description = "repair run"), (status = 404, description = "not found"))
)]
pub(crate) async fn get_master_repair_run(
    State(state): State<AppState>,
    AxumPath((proj_id, run_id)): AxumPath<(i64, String)>,
) -> Result<Json<Value>, ApiError> {
    ensure_master_role(&state, proj_id).await?;
    let run = state
        .session_db
        .get_repair_run(&run_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "repair run not found"))?;
    if run.master_proj_id != proj_id {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "repair run not found"));
    }
    Ok(Json(json!({"run": run})))
}

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/schedules",
    tag = "Master",
    operation_id = "list_master_schedules",
    params(("proj_id" = i64, Path, description = "Master project id")),
    responses((status = 200, description = "schedules"), (status = 400, description = "not master"))
)]
pub(crate) async fn list_master_schedules(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<Value>, ApiError> {
    ensure_master_role(&state, proj_id).await?;
    let jobs = state
        .session_db
        .list_scheduled_jobs(Some(proj_id))
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(json!({"jobs": jobs})))
}

#[utoipa::path(
    put,
    path = "/v1/projects/{proj_id}/schedules",
    tag = "Master",
    operation_id = "put_master_schedule",
    params(("proj_id" = i64, Path, description = "Master project id")),
    request_body = PutScheduleRequest,
    responses((status = 200, description = "saved schedule"), (status = 400, description = "not master"))
)]
pub(crate) async fn put_master_schedule(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Json(req): Json<PutScheduleRequest>,
) -> Result<Json<Value>, ApiError> {
    ensure_master_role(&state, proj_id).await?;
    let now = now_ms();
    let job_id = req
        .job_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(master_observer::new_scheduled_job_id);
    let prompt = req.prompt_template.unwrap_or_else(|| {
        if req.schedule_kind == "weekly" {
            master_scheduler::default_weekly_repair_prompt_template()
        } else {
            master_scheduler::default_daily_prompt_template()
        }
    });
    let job = master_observer::GatewayScheduledJobRow {
        job_id: job_id.clone(),
        master_proj_id: proj_id,
        schedule_kind: req.schedule_kind,
        run_at_hhmm: req.run_at_hhmm,
        weekday: req.weekday,
        enabled: req.enabled,
        prompt_template: prompt,
        last_run_at_ms: None,
        last_task_id: None,
        last_error: None,
        created_at_ms: now,
        updated_at_ms: now,
    };
    // Preserve last_* if updating existing.
    let mut job = job;
    if let Ok(existing) = state.session_db.list_scheduled_jobs(Some(proj_id)).await {
        if let Some(old) = existing.into_iter().find(|j| j.job_id == job_id) {
            job.last_run_at_ms = old.last_run_at_ms;
            job.last_task_id = old.last_task_id;
            job.last_error = old.last_error;
            job.created_at_ms = old.created_at_ms;
        }
    }
    state
        .session_db
        .upsert_scheduled_job(&job)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(json!({"job": job})))
}

#[utoipa::path(
    delete,
    path = "/v1/projects/{proj_id}/schedules/{job_id}",
    tag = "Master",
    operation_id = "delete_master_schedule",
    params(
        ("proj_id" = i64, Path, description = "Master project id"),
        ("job_id" = String, Path, description = "Job id")
    ),
    responses((status = 200, description = "deleted"), (status = 400, description = "not master"))
)]
pub(crate) async fn delete_master_schedule(
    State(state): State<AppState>,
    AxumPath((proj_id, job_id)): AxumPath<(i64, String)>,
) -> Result<Json<Value>, ApiError> {
    ensure_master_role(&state, proj_id).await?;
    let ok = state
        .session_db
        .delete_scheduled_job(&job_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(json!({"deleted": ok, "jobId": job_id})))
}

/// Manual kickoff: same path as the minute ticker (`fire_job`). Author: kejiqing
#[utoipa::path(
    post,
    path = "/v1/projects/{proj_id}/schedules/{job_id}/run",
    tag = "Master",
    operation_id = "run_master_schedule",
    params(
        ("proj_id" = i64, Path, description = "Master project id"),
        ("job_id" = String, Path, description = "Scheduled job id")
    ),
    responses(
        (status = 200, description = "Enqueued master solve for this schedule"),
        (status = 400, description = "Not master / fire failed"),
        (status = 404, description = "Job not found")
    )
)]
pub(crate) async fn run_master_schedule(
    State(state): State<AppState>,
    AxumPath((proj_id, job_id)): AxumPath<(i64, String)>,
) -> Result<Json<Value>, ApiError> {
    ensure_master_role(&state, proj_id).await?;
    let job = master_scheduler::run_scheduled_job_now(&state, proj_id, &job_id)
        .await
        .map_err(|e| {
            if e.contains("not found") {
                ApiError::new(StatusCode::NOT_FOUND, e)
            } else {
                ApiError::new(StatusCode::BAD_REQUEST, e)
            }
        })?;
    Ok(Json(json!({
        "job": job,
        "taskId": job.last_task_id,
        "enqueued": true,
    })))
}

#[utoipa::path(
    post,
    path = "/v1/master/{master_proj_id}/mcp",
    tag = "Master",
    operation_id = "master_mcp_http_handler",
    params(("master_proj_id" = i64, Path, description = "Master project id")),
    request_body(content = inline(Object), content_type = "application/json", description = "MCP JSON-RPC request body"),
    responses(
        (status = 200, description = "MCP JSON-RPC response", content_type = "application/json"),
        (status = 401, description = "unauthorized")
    )
)]
pub(crate) async fn master_mcp_http_handler(
    State(state): State<AppState>,
    AxumPath(master_proj_id): AxumPath<i64>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let backend = GatewayAdminMcpSolveBackend {
        state: state.clone(),
    };
    master_mcp::handle_master_mcp_post(
        &state.session_db,
        &backend,
        master_proj_id,
        &headers,
        body,
    )
    .await
}

/// Used by master_scheduler ticker. Author: kejiqing
pub async fn master_scheduler_enqueue_solve(
    state: AppState,
    input: admin_mcp_solve::AdminMcpSolveInput,
) -> Result<Value, String> {
    use admin_mcp_solve::AdminMcpSolveBackend;
    let backend = GatewayAdminMcpSolveBackend { state };
    backend.gateway_solve_async(input).await
}

/// Public dispatch alias (compat). Author: kejiqing
pub async fn master_mcp_dispatch(
    state: AppState,
    master_proj_id: i64,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    master_mcp_http_handler(State(state), AxumPath(master_proj_id), headers, body).await
}
