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
    /// Legacy: local-only proj ids. Author: kejiqing
    #[serde(rename = "apprenticeProjIds", default)]
    apprentice_proj_ids: Vec<i64>,
    /// Preferred: proj id + optional gateway base (empty = this gateway). Author: kejiqing
    #[serde(default)]
    apprentices: Vec<ApprenticeSpec>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub(crate) struct ApprenticeSpec {
    #[serde(rename = "apprenticeProjId")]
    apprentice_proj_id: i64,
    /// Empty / omitted = local. IP, host:port, or http(s) URL. Author: kejiqing
    #[serde(rename = "gatewayBase", default)]
    gateway_base: String,
    /// Peer gateway `CLAW_MASTER_MCP_TOKEN`. Omit on update to keep existing. Author: kejiqing
    #[serde(rename = "mcpToken", default)]
    mcp_token: Option<String>,
}

fn desired_apprentice_specs(req: PutApprenticesRequest) -> Result<Vec<ApprenticeSpec>, ApiError> {
    let mut out: Vec<ApprenticeSpec> = if !req.apprentices.is_empty() {
        req.apprentices
    } else {
        req.apprentice_proj_ids
            .into_iter()
            .map(|id| ApprenticeSpec {
                apprentice_proj_id: id,
                gateway_base: String::new(),
                mcp_token: None,
            })
            .collect()
    };
    let mut seen = std::collections::HashSet::new();
    for spec in &mut out {
        let base = gateway_endpoint::parse_apprentice_gateway_base(&spec.gateway_base).map_err(
            |e| ApiError::new(StatusCode::BAD_REQUEST, e),
        )?;
        spec.gateway_base = base;
        if let Some(ref t) = spec.mcp_token {
            spec.mcp_token = Some(t.trim().to_string());
        }
        if !seen.insert(spec.apprentice_proj_id) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "duplicate apprenticeProjId {}",
                    spec.apprentice_proj_id
                ),
            ));
        }
    }
    Ok(out)
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
    } else if role == master_observer::PROJECT_ROLE_ROUTER {
        master_observer::seed_router_project(&state.session_db, proj_id)
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
    let specs = desired_apprentice_specs(req)?;
    let desired: std::collections::HashMap<i64, ApprenticeSpec> = specs
        .into_iter()
        .map(|s| (s.apprentice_proj_id, s))
        .collect();
    let existing = state
        .session_db
        .list_master_links(master_proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    for link in &existing {
        if !desired.contains_key(&link.apprentice_proj_id) && !link.orphaned {
            state
                .session_db
                .mark_master_link_orphaned(master_proj_id, link.apprentice_proj_id)
                .await
                .map_err(|e| session_db_err(&e))?;
        }
    }
    let self_base = state.gateway_identity.gateway_base.as_str();
    for (aid, spec) in &desired {
        if *aid == master_proj_id {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "master cannot apprentice itself",
            ));
        }
        if let Some(link) = existing.iter().find(|l| l.apprentice_proj_id == *aid) {
            let mut restored = link.clone();
            restored.orphaned = false;
            restored.apprentice_gateway_base = spec.gateway_base.clone();
            if let Some(ref tok) = spec.mcp_token {
                restored.apprentice_mcp_token = tok.clone();
            }
            restored.mcp_token_set = !restored.apprentice_mcp_token.trim().is_empty();
            restored.updated_at_ms = now_ms();
            // Remote gateway requires a stored peer token. Author: kejiqing
            let probe = restored.clone();
            if master_apprentice_access::link_peer_base(&probe, self_base).is_some()
                && restored.apprentice_mcp_token.trim().is_empty()
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "apprentice {aid}: mcpToken required when gatewayBase is remote"
                    ),
                ));
            }
            state
                .session_db
                .upsert_master_link(&restored)
                .await
                .map_err(|e| session_db_err(&e))?;
            continue;
        }
        let token = spec.mcp_token.clone().unwrap_or_default();
        create_observation_for_apprentice(
            &state,
            master_proj_id,
            *aid,
            &spec.gateway_base,
            &token,
            self_base,
        )
        .await?;
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
    gateway_base: &str,
    mcp_token: &str,
    self_gateway_base: &str,
) -> Result<(), ApiError> {
    let _ = master_apprentice_access::assert_apprentice_pairable(
        &state.session_db,
        self_gateway_base,
        apprentice_proj_id,
        gateway_base,
        mcp_token,
    )
    .await
    .map_err(|e| {
        if e.contains("not found") || e.contains("no stable") || e.contains("404") {
            ApiError::new(StatusCode::NOT_FOUND, e)
        } else {
            ApiError::new(StatusCode::BAD_REQUEST, e)
        }
    })?;

    // Shadow observation is co-located with the apprentice gateway. Author: kejiqing
    let mut probe = master_observer::ProjectMasterLinkRow {
        master_proj_id,
        apprentice_proj_id,
        observation_proj_id: 0,
        apprentice_gateway_base: gateway_base.to_string(),
        apprentice_mcp_token: mcp_token.to_string(),
        mcp_token_set: !mcp_token.trim().is_empty(),
        orphaned: false,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    let obs_id = if let Some(peer) =
        master_apprentice_access::link_peer_base(&probe, self_gateway_base)
    {
        let token = master_apprentice_access::peer_auth_token(&probe)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
        master_apprentice_access::create_observation_on_peer(
            &peer,
            &token,
            master_proj_id,
            apprentice_proj_id,
        )
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?
    } else {
        create_observation_space_local(state, master_proj_id, apprentice_proj_id).await?
    };

    let now = now_ms();
    probe.observation_proj_id = obs_id;
    probe.orphaned = false;
    probe.created_at_ms = now;
    probe.updated_at_ms = now;
    state
        .session_db
        .upsert_master_link(&probe)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(())
}

/// Create observation project on **this** gateway (apprentice must be local). Author: kejiqing
pub(crate) async fn create_observation_space_local(
    state: &AppState,
    master_proj_id: i64,
    apprentice_proj_id: i64,
) -> Result<i64, ApiError> {
    let source = project_config_draft::row_for_materialize(&state.session_db, apprentice_proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("apprentice {apprentice_proj_id} has no stable config"),
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

    let obs_id = resolve_create_proj_id(state, None).await?;
    let code = format!("obs-m{}-a{}", master_proj_id, apprentice_proj_id);
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
    Ok(obs_id)
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
        state.gateway_identity.gateway_base.as_str(),
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

fn master_peer_auth(headers: &HeaderMap) -> Result<(), ApiError> {
    master_apprentice_access::verify_master_peer_auth(headers)
        .map_err(|e| ApiError::new(StatusCode::UNAUTHORIZED, e))
}

#[utoipa::path(
    get,
    path = "/v1/master-peer/projects/{proj_id}/stable-config",
    tag = "Master",
    operation_id = "master_peer_stable_config",
    params(("proj_id" = i64, Path, description = "Apprentice project id on this gateway")),
    responses((status = 200, description = "stable config"), (status = 401, description = "unauthorized"))
)]
pub(crate) async fn master_peer_stable_config(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    master_peer_auth(&headers)?;
    let role = state
        .session_db
        .get_project_role(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let row = project_config_draft::row_for_materialize(&state.session_db, proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("apprentice {proj_id} has no stable config"),
            )
        })?;
    let dto = master_apprentice_access::row_to_stable_dto(&row, &role);
    Ok(Json(serde_json::to_value(dto).unwrap_or(json!({}))))
}

#[derive(Debug, Deserialize)]
pub(crate) struct MasterPeerSessionsQuery {
    limit: Option<i64>,
    #[serde(rename = "updatedAfterMs")]
    updated_after_ms: Option<i64>,
    #[serde(rename = "updatedBeforeMs")]
    updated_before_ms: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/v1/master-peer/projects/{proj_id}/sessions",
    tag = "Master",
    operation_id = "master_peer_sessions",
    params(("proj_id" = i64, Path, description = "Apprentice project id")),
    responses((status = 200, description = "sessions"))
)]
pub(crate) async fn master_peer_sessions(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Query(q): Query<MasterPeerSessionsQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    master_peer_auth(&headers)?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let link = master_observer::ProjectMasterLinkRow {
        master_proj_id: 0,
        apprentice_proj_id: proj_id,
        observation_proj_id: 0,
        apprentice_gateway_base: String::new(),
        apprentice_mcp_token: String::new(),
        mcp_token_set: false,
        orphaned: false,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    let v = master_apprentice_access::list_apprentice_sessions(
        &state.session_db,
        state.gateway_identity.gateway_base.as_str(),
        &link,
        limit,
        q.updated_after_ms,
        q.updated_before_ms,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(v))
}

#[utoipa::path(
    get,
    path = "/v1/master-peer/projects/{proj_id}/sessions/{session_id}/turns",
    tag = "Master",
    operation_id = "master_peer_session_turns",
    params(
        ("proj_id" = i64, Path, description = "Apprentice project id"),
        ("session_id" = String, Path, description = "Session id")
    ),
    responses((status = 200, description = "turns"))
)]
pub(crate) async fn master_peer_session_turns(
    State(state): State<AppState>,
    AxumPath((proj_id, session_id)): AxumPath<(i64, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    master_peer_auth(&headers)?;
    let link = master_observer::ProjectMasterLinkRow {
        master_proj_id: 0,
        apprentice_proj_id: proj_id,
        observation_proj_id: 0,
        apprentice_gateway_base: String::new(),
        apprentice_mcp_token: String::new(),
        mcp_token_set: false,
        orphaned: false,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    let v = master_apprentice_access::list_apprentice_turns(
        &state.session_db,
        state.gateway_identity.gateway_base.as_str(),
        &link,
        &session_id,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(v))
}

#[derive(Debug, Deserialize)]
pub(crate) struct MasterPeerReplayTurnQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "turnId")]
    turn_id: String,
}

#[utoipa::path(
    get,
    path = "/v1/master-peer/projects/{proj_id}/replay-turn",
    tag = "Master",
    operation_id = "master_peer_replay_turn",
    params(("proj_id" = i64, Path, description = "Apprentice project id")),
    responses((status = 200, description = "turn for replay"))
)]
pub(crate) async fn master_peer_replay_turn(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Query(q): Query<MasterPeerReplayTurnQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    master_peer_auth(&headers)?;
    let found = state
        .session_db
        .get_turn_for_replay(&q.session_id, proj_id, &q.turn_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    match found {
        Some((prompt, entry)) => Ok(Json(json!({
            "found": true,
            "userPrompt": prompt,
            "entryParams": entry,
        }))),
        None => Ok(Json(json!({"found": false}))),
    }
}

#[utoipa::path(
    put,
    path = "/v1/master-peer/projects/{proj_id}/draft",
    tag = "Master",
    operation_id = "master_peer_put_draft",
    params(("proj_id" = i64, Path, description = "Apprentice project id")),
    responses((status = 200, description = "draft updated"))
)]
pub(crate) async fn master_peer_put_draft(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    headers: HeaderMap,
    Json(patch): Json<master_apprentice_access::ApprenticeDraftPutDto>,
) -> Result<Json<Value>, ApiError> {
    master_peer_auth(&headers)?;
    master_apprentice_access::apply_draft_patch_local(&state.session_db, proj_id, &patch)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({"projId": proj_id, "draftOpen": true})))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct MasterPeerCreateObservationRequest {
    #[serde(rename = "masterProjId")]
    master_proj_id: i64,
}

#[utoipa::path(
    post,
    path = "/v1/master-peer/projects/{proj_id}/observation",
    tag = "Master",
    operation_id = "master_peer_create_observation",
    params(("proj_id" = i64, Path, description = "Apprentice project id on this gateway")),
    responses((status = 200, description = "created observation proj id"))
)]
pub(crate) async fn master_peer_create_observation(
    State(state): State<AppState>,
    AxumPath(apprentice_proj_id): AxumPath<i64>,
    headers: HeaderMap,
    Json(req): Json<MasterPeerCreateObservationRequest>,
) -> Result<Json<Value>, ApiError> {
    master_peer_auth(&headers)?;
    let obs_id =
        create_observation_space_local(&state, req.master_proj_id, apprentice_proj_id).await?;
    Ok(Json(json!({ "observationProjId": obs_id })))
}

#[utoipa::path(
    post,
    path = "/v1/master-peer/observations/{observation_proj_id}/sync-from/{apprentice_proj_id}",
    tag = "Master",
    operation_id = "master_peer_sync_observation",
    params(
        ("observation_proj_id" = i64, Path, description = "Observation project id"),
        ("apprentice_proj_id" = i64, Path, description = "Apprentice project id")
    ),
    responses((status = 200, description = "synced"))
)]
pub(crate) async fn master_peer_sync_observation(
    State(state): State<AppState>,
    AxumPath((observation_proj_id, apprentice_proj_id)): AxumPath<(i64, i64)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    master_peer_auth(&headers)?;
    let link = master_observer::ProjectMasterLinkRow {
        master_proj_id: 0,
        apprentice_proj_id,
        observation_proj_id,
        apprentice_gateway_base: String::new(),
        apprentice_mcp_token: String::new(),
        mcp_token_set: false,
        orphaned: false,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    let (before, after, baseline) = master_apprentice_access::sync_observation_from_apprentice(
        &state.session_db,
        state.gateway_identity.gateway_base.as_str(),
        &link,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({
        "beforeContentRev": before,
        "afterContentRev": after,
        "baselineApprenticeContentRev": baseline,
    })))
}

#[utoipa::path(
    put,
    path = "/v1/master-peer/observations/{observation_proj_id}/draft",
    tag = "Master",
    operation_id = "master_peer_observation_draft",
    params(("observation_proj_id" = i64, Path, description = "Observation project id")),
    responses((status = 200, description = "draft updated"))
)]
pub(crate) async fn master_peer_observation_draft(
    State(state): State<AppState>,
    AxumPath(observation_proj_id): AxumPath<i64>,
    headers: HeaderMap,
    Json(patch): Json<master_apprentice_access::ObservationDraftPutDto>,
) -> Result<Json<Value>, ApiError> {
    master_peer_auth(&headers)?;
    let rev = master_apprentice_access::apply_observation_draft_local(
        &state.session_db,
        observation_proj_id,
        &patch,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(json!({
        "observationProjId": observation_proj_id,
        "stableContentRev": rev,
    })))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct MasterPeerObservationSolveRequest {
    #[serde(rename = "userPrompt")]
    user_prompt: String,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "extraSession")]
    extra_session: Option<Value>,
}

#[utoipa::path(
    post,
    path = "/v1/master-peer/observations/{observation_proj_id}/solve",
    tag = "Master",
    operation_id = "master_peer_observation_solve",
    params(("observation_proj_id" = i64, Path, description = "Observation project id")),
    responses((status = 200, description = "solve enqueued"))
)]
pub(crate) async fn master_peer_observation_solve(
    State(state): State<AppState>,
    AxumPath(observation_proj_id): AxumPath<i64>,
    headers: HeaderMap,
    Json(req): Json<MasterPeerObservationSolveRequest>,
) -> Result<Json<Value>, ApiError> {
    master_peer_auth(&headers)?;
    let role = state
        .session_db
        .get_project_role(observation_proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if role != master_observer::PROJECT_ROLE_OBSERVATION {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("proj {observation_proj_id} is not observation (role={role})"),
        ));
    }
    let input = admin_mcp_solve::AdminMcpSolveInput {
        proj_id: observation_proj_id,
        user_prompt: req.user_prompt,
        session_id: req.session_id,
        model: None,
        timeout_seconds: None,
        extra_session: req.extra_session,
        allowed_tools: None,
        max_iterations: None,
        attachments: None,
    };
    admin_mcp_solve::validate_admin_mcp_solve_input(&state.session_db, &input)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    use admin_mcp_solve::AdminMcpSolveBackend;
    let backend = GatewayAdminMcpSolveBackend { state };
    let resp = backend
        .gateway_solve_async(input)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(resp))
}
