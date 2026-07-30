// Fragment of routes::app (include!). Author: kejiqing

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/agent/ws",
    tag = "Sessions",
    operation_id = "agent_ws_handler",
    params(
        ("session_id" = String, Path, description = "Gateway session id"),
        session_agent_api::AgentProjQuery
    ),
    responses(
        (status = 101, description = "WebSocket upgrade for OVS agent chat bridge"),
        (status = 400, description = "Invalid projId or session"),
        (status = 409, description = "Concurrent prompt on same record session")
    )
)]
pub(crate) async fn agent_ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(q): Query<session_agent_api::AgentProjQuery>,
) -> impl IntoResponse {
    session_agent_api::agent_ws_upgrade(state.terminal_api_ctx(), session_id, q, ws).await
}

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/ovs/workspace",
    tag = "Inference",
    operation_id = "ovs_workspace_handler",
    params(
        ("proj_id" = i64, Path, description = "Project id")
    ),
    responses(
        (status = 200, description = "OVS workspace metadata", body = session_ovs_api::OvsWorkspaceResponse),
        (status = 404, description = "Project not found")
    )
)]
pub(crate) async fn ovs_workspace_handler(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<session_ovs_api::OvsWorkspaceResponse>, session_ovs_api::OvsApiError> {
    session_ovs_api::get_ovs_workspace(
        state.ovs_api_ctx(),
        &state.session_db,
        Some(state.pool_clients.e2b_worker_registry()),
        proj_id,
    )
    .await
}

pub(crate) fn progress_poll_interval_ms() -> u64 {
    std::env::var("CLAW_TASK_PROGRESS_POLL_MS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|&n| n >= 100)
        .unwrap_or(400)
}

pub(crate) fn gateway_queue_snapshot(tasks: &HashMap<String, TaskInner>) -> task_status::GatewayQueueSnapshot {
    let rows: HashMap<String, TaskStatusRow> = tasks
        .iter()
        .map(|(id, inner)| {
            (
                id.clone(),
                TaskStatusRow {
                    status: inner.record.status.clone(),
                },
            )
        })
        .collect();
    count_gateway_tasks(&rows)
}

pub(crate) async fn resolve_session_home_path(
    state: &AppState,
    proj_id: i64,
    session_id: &str,
) -> Option<PathBuf> {
    let rel = state
        .session_db
        .get_session_home_rel(session_id, proj_id)
        .await
        .ok()??;
    session_merge::validate_session_home_rel(&rel).ok()?;
    Some(join_session_home(&state.cfg.work_root, &rel))
}

pub(crate) async fn load_turn_progress_snapshot(
    state: &AppState,
    turn_id: &str,
    session_id: &str,
    proj_id: i64,
    status: &str,
    limit: usize,
) -> Result<pool_consumer_resolve::TurnProgressSnapshot, ApiError> {
    if let Ok(pool) = state
        .pool_clients
        .pool_for_turn(&state.session_db, turn_id, session_id, proj_id)
        .await
    {
        pool_consumer_resolve::maybe_sync_running_turn_progress_from_worker(&pool, turn_id, status)
            .await;
    }
    pool_consumer_resolve::resolve_turn_progress(&state.session_db, turn_id, limit)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))
}

pub(crate) async fn refresh_task_progress(state: &AppState, task_id: &str) {
    let snapshot = {
        let (status, proj_id, session_id, turn_id) = {
            let tasks = state.tasks.lock().await;
            let Some(inner) = tasks.get(task_id) else {
                return;
            };
            (
                inner.record.status.clone(),
                inner.proj_id,
                inner.record.session_id.clone(),
                inner.record.turn_id.clone(),
            )
        };
        let session_home = resolve_session_home_path(state, proj_id, &session_id).await;
        let queue = {
            let tasks = state.tasks.lock().await;
            gateway_queue_snapshot(&tasks)
        };
        let trace_paths = session_home
            .as_ref()
            .map(|home| discover_trace_paths(home, &state.cfg.work_root, &session_id))
            .unwrap_or_default();
        let tool = trace_tail_suggests_tool_call(&trace_paths);
        let progress_snap =
            load_turn_progress_snapshot(state, &turn_id, &session_id, proj_id, &status, 50)
                .await
                .unwrap_or_default();
        let desc =
            resolve_current_task_desc(&status, &queue, tool, progress_snap.task_progress.as_ref());
        let updated_ms = progress_snap
            .task_progress
            .as_ref()
            .map(|p| p.updated_at_ms);
        let (plan_title, todos) = pool_consumer_resolve::plan_fields_from_snapshot(&progress_snap);
        (desc, updated_ms, progress_snap.events, plan_title, todos)
    };
    let mut tasks = state.tasks.lock().await;
    if let Some(inner) = tasks.get_mut(task_id) {
        inner.record.current_task_desc = snapshot.0;
        inner.record.progress_updated_at_ms = snapshot.1;
        inner.record.progress_history = snapshot.2;
        inner.record.plan_title = snapshot.3;
        inner.record.todos = snapshot.4;
    }
}

pub(crate) fn spawn_task_progress_poller(state: AppState, task_id: String) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(progress_poll_interval_ms()));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let active = {
                let tasks = state.tasks.lock().await;
                tasks.get(&task_id).is_some_and(|inner| {
                    matches!(inner.record.status.as_str(), "queued" | "running")
                })
            };
            if !active {
                break;
            }
            refresh_task_progress(&state, &task_id).await;
        }
    });
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SessionExecutionQuery {
    #[serde(rename = "proj_id")]
    #[param(rename = "proj_id")]
    proj_id: i64,
    #[serde(default)]
    include_trace: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListProjectSessionsQuery {
    #[serde(default = "default_session_list_limit")]
    limit: i64,
    /// Keyset: load rows strictly older than this `(updatedAtMs, sessionId)` pair.
    #[serde(rename = "beforeUpdatedAtMs")]
    #[param(rename = "beforeUpdatedAtMs")]
    before_updated_at_ms: Option<i64>,
    #[serde(rename = "beforeSessionId")]
    #[param(rename = "beforeSessionId")]
    before_session_id: Option<String>,
    #[serde(rename = "updatedFromMs")]
    #[param(rename = "updatedFromMs")]
    updated_from_ms: Option<i64>,
    #[serde(rename = "updatedToMs")]
    #[param(rename = "updatedToMs")]
    updated_to_ms: Option<i64>,
    /// Fuzzy match on first-turn `user_prompt` (ILIKE).
    q: Option<String>,
    /// `T_<32 hex>` → session owning that turn; otherwise `session_id` ILIKE substring.
    #[serde(rename = "sessionId")]
    #[param(rename = "sessionId")]
    session_id: Option<String>,
    /// URL-encoded JSON object: only keys in `project_config.extra_session_fields_json`; ILIKE per field on any turn's `entry_params_json.extraSession`.
    #[serde(rename = "extraSession")]
    #[param(rename = "extraSession")]
    extra_session: Option<String>,
}

pub(crate) fn default_session_list_limit() -> i64 {
    20
}

pub(crate) async fn parse_list_sessions_extra_session_filter(
    state: &AppState,
    proj_id: i64,
    raw: Option<String>,
) -> Result<Option<BTreeMap<String, String>>, ApiError> {
    let Some(raw) = raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&raw).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "extraSession query must be a JSON object",
        )
    })?;
    let fields_json = match state.session_db.get_project_config(proj_id).await {
        Ok(Some(row)) => row.extra_session_fields_json,
        Ok(None) => json!([]),
        Err(e) => return Err(session_db_err(&e)),
    };
    let allowed = project_extra_session::parse_extra_session_fields_json(&fields_json)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let map = project_extra_session::parse_extra_session_search_filter(Some(&value), &allowed)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if map.is_empty() {
        return Ok(None);
    }
    Ok(Some(map))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct GatewaySessionSummaryJson {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "createdAtMs")]
    created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: i64,
    #[serde(rename = "turnCount")]
    turn_count: i64,
    #[serde(rename = "previewPrompt")]
    preview_prompt: Option<String>,
    #[serde(rename = "clientOrigin", skip_serializing_if = "Option::is_none")]
    client_origin: Option<String>,
    #[serde(rename = "hasBadFeedback")]
    has_bad_feedback: bool,
    #[serde(rename = "hasGoodFeedback")]
    has_good_feedback: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ListProjectSessionsResponse {
    #[serde(rename = "projId")]
    proj_id: i64,
    sessions: Vec<GatewaySessionSummaryJson>,
    #[serde(rename = "hasMore")]
    has_more: bool,
}

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/sessions",
    tag = "Sessions",
    operation_id = "list_project_sessions",
    params(
        ("proj_id" = i64, Path, description = "Project id"),
        ListProjectSessionsQuery
    ),
    responses(
        (status = 200, description = "Paginated session list", body = ListProjectSessionsResponse),
        (status = 400, description = "Invalid projId or extraSession filter")
    )
)]
pub(crate) async fn list_project_sessions(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Query(query): Query<ListProjectSessionsQuery>,
) -> Result<Json<ListProjectSessionsResponse>, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let extra_filter =
        parse_list_sessions_extra_session_filter(&state, proj_id, query.extra_session).await?;
    let limit = query.limit;
    let rows = state
        .session_db
        .list_sessions_for_proj(
            proj_id,
            limit,
            query.before_updated_at_ms,
            query.before_session_id.as_deref(),
            query.updated_from_ms,
            query.updated_to_ms,
            query.q.as_deref(),
            query.session_id.as_deref(),
            extra_filter.as_ref(),
        )
        .await
        .map_err(|e| session_db_err(&e))?;
    let has_more = i64::try_from(rows.len()).unwrap_or(0) >= limit;
    Ok(Json(ListProjectSessionsResponse {
        proj_id,
        sessions: rows
            .into_iter()
            .map(|r| GatewaySessionSummaryJson {
                session_id: r.session_id,
                created_at_ms: r.created_at_ms,
                updated_at_ms: r.updated_at_ms,
                turn_count: r.turn_count,
                preview_prompt: r.preview_prompt,
                client_origin: r.client_origin,
                has_bad_feedback: r.has_bad_feedback,
                has_good_feedback: r.has_good_feedback,
            })
            .collect(),
        has_more,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/execution",
    tag = "Sessions",
    operation_id = "get_session_execution",
    params(
        ("session_id" = String, Path, description = "Gateway session id"),
        SessionExecutionQuery
    ),
    responses(
        (status = 200, description = "Session execution snapshot", body = SessionExecutionResponse),
        (status = 404, description = "Session not found")
    )
)]
pub(crate) async fn get_session_execution(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<SessionExecutionQuery>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<SessionExecutionResponse>, ApiError> {
    if query.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "proj_id must be >= 1",
        ));
    }
    let session_home_rel = state
        .session_db
        .get_session_home_rel(&session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("session not found: {session_id} proj_id={}", query.proj_id),
            )
        })?;
    session_merge::validate_session_home_rel(&session_home_rel).map_err(session_routing_error)?;
    let session_home = join_session_home(&state.cfg.work_root, &session_home_rel);

    refresh_task_progress(&state, &session_id).await;

    let (record_opt, queue) = {
        let tasks = state.tasks.lock().await;
        let queue = gateway_queue_snapshot(&tasks);
        let record = tasks.get(&session_id).map(|inner| inner.record.clone());
        (record, queue)
    };
    let turn_id = record_opt
        .as_ref()
        .map(|r| r.turn_id.clone())
        .unwrap_or_default();
    let task_status_for_progress = record_opt
        .as_ref()
        .map_or_else(|| "unknown".to_string(), |r| r.status.clone());
    let task_snapshot = if let Some(ref record) = record_opt {
        let has_report = task_has_report(&state, record).await;
        let report_time_ms = task_report_time_ms(&state, record).await;
        Some(SessionExecutionTask {
            task_id: record.task_id.clone(),
            status: record.status.clone(),
            has_report,
            report_time_ms,
            created_at_ms: record.created_at_ms,
            started_at_ms: record.started_at_ms,
            finished_at_ms: record.finished_at_ms,
            current_task_desc: record.current_task_desc.clone(),
        })
    } else {
        None
    };

    let task = task_snapshot.unwrap_or_else(|| SessionExecutionTask {
        task_id: session_id.clone(),
        status: "unknown".to_string(),
        has_report: false,
        report_time_ms: None,
        created_at_ms: 0,
        started_at_ms: None,
        finished_at_ms: None,
        current_task_desc: None,
    });

    let progress_snap = load_turn_progress_snapshot(
        &state,
        &turn_id,
        &session_id,
        query.proj_id,
        &task_status_for_progress,
        50,
    )
    .await?;
    let progress = progress_snap.task_progress.clone();
    let progress_history = progress_snap.events;
    let trace_paths = discover_trace_paths(&session_home, &state.cfg.work_root, &session_id);
    let trace_tail = if query.include_trace {
        read_trace_tail(&trace_paths, 50, true)
    } else {
        read_trace_tail(&trace_paths, 20, false)
    };

    info!(
        request_id = %http_request_id.0,
        session_id = %session_id,
        proj_id = query.proj_id,
        endpoint = "/v1/sessions/{session_id}/execution",
        "gateway_session_execution"
    );

    Ok(Json(SessionExecutionResponse {
        session_id,
        proj_id: query.proj_id,
        session_home_rel,
        task,
        progress,
        progress_history,
        queue,
        trace_tail,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/translate",
    tag = "Sessions",
    operation_id = "post_gateway_translate",
    summary = "Translate text with the active gateway LLM",
    description = "Translates a single text block using the LLM currently activated in Admin global settings. Input is capped at 8000 characters; the response carries the translation body only.",
    request_body = gateway_translate::GatewayTranslateRequest,
    responses(
        (status = 200, description = "Translated text", body = gateway_translate::GatewayTranslateResponse),
        (status = 400, description = "text is empty or exceeds 8000 characters"),
        (status = 503, description = "No active LLM configured in Admin, or its apiKey is missing"),
        (status = 502, description = "Upstream LLM call failed or returned an empty translation")
    )
)]
pub(crate) async fn post_gateway_translate(
    State(state): State<AppState>,
    Json(body): Json<gateway_translate::GatewayTranslateRequest>,
) -> Result<Json<gateway_translate::GatewayTranslateResponse>, ApiError> {
    gateway_translate::post_gateway_translate_handler(&state.session_db, body)
        .await
        .map_err(|e| ApiError::new(e.status, e.message))
}

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/conversation_translate",
    tag = "Sessions",
    operation_id = "get_conversation_translate",
    params(
        ("session_id" = String, Path, description = "Gateway session id"),
        ListSessionTurnsQuery
    ),
    responses(
        (status = 200, description = "Cached whole-conversation translation snapshot", body = gateway_translate::GetConversationTranslateResponse),
        (status = 404, description = "Session not found")
    )
)]
pub(crate) async fn get_conversation_translate(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<ListSessionTurnsQuery>,
) -> Result<Json<gateway_translate::GetConversationTranslateResponse>, ApiError> {
    gateway_translate::get_conversation_translate_handler(
        &state.session_db,
        &session_id,
        query.proj_id,
    )
    .await
    .map_err(|e| ApiError::new(e.status, e.message))
}

#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/conversation_translate",
    tag = "Sessions",
    operation_id = "rebuild_conversation_translate",
    params(
        ("session_id" = String, Path, description = "Gateway session id"),
        ListSessionTurnsQuery
    ),
    responses(
        (status = 200, description = "Rebuild accepted; poll GET for completion", body = gateway_translate::RebuildConversationTranslateResponse),
        (status = 400, description = "No completed turns to translate"),
        (status = 409, description = "Translation already in progress")
    )
)]
pub(crate) async fn rebuild_conversation_translate(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<ListSessionTurnsQuery>,
) -> Result<Json<gateway_translate::RebuildConversationTranslateResponse>, ApiError> {
    gateway_translate::rebuild_conversation_translate_handler(
        state.session_db.clone(),
        session_id,
        query.proj_id,
    )
    .await
    .map_err(|e| ApiError::new(e.status, e.message))
}

