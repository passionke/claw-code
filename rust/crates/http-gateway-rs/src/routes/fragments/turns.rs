// Fragment of routes::app (include!). Author: kejiqing

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListSessionTurnsQuery {
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    #[param(rename = "projId")]
    proj_id: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct GatewayTurnSummaryJson {
    #[serde(rename = "turnId")]
    turn_id: String,
    #[serde(rename = "userPrompt")]
    user_prompt: Option<String>,
    status: String,
    #[serde(rename = "createdAtMs")]
    created_at_ms: i64,
    #[serde(rename = "finishedAtMs")]
    finished_at_ms: Option<i64>,
    #[serde(rename = "hasReport")]
    has_report: bool,
    #[serde(rename = "reportBody", skip_serializing_if = "Option::is_none")]
    report_body: Option<String>,
    #[serde(rename = "failureDetail", skip_serializing_if = "Option::is_none")]
    failure_detail: Option<String>,
    #[serde(rename = "clientOrigin", skip_serializing_if = "Option::is_none")]
    client_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    feedback: Option<String>,
    #[serde(rename = "extraSession", skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    extra_session: Option<Value>,
    #[serde(rename = "attachments", skip_serializing_if = "Option::is_none")]
    attachments: Option<Vec<session_upload::SessionUploadedAttachment>>,
    #[serde(rename = "poolId", skip_serializing_if = "Option::is_none")]
    pool_id: Option<String>,
    #[serde(rename = "workerName", skip_serializing_if = "Option::is_none")]
    worker_name: Option<String>,
    #[serde(rename = "workerProfile", skip_serializing_if = "Option::is_none")]
    worker_profile: Option<String>,
    #[serde(rename = "workerExecUser", skip_serializing_if = "Option::is_none")]
    worker_exec_user: Option<String>,
    #[serde(rename = "gatewayId", skip_serializing_if = "Option::is_none")]
    gateway_id: Option<String>,
    #[serde(rename = "gatewayBase", skip_serializing_if = "Option::is_none")]
    gateway_base: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ListSessionTurnsResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "projId")]
    proj_id: i64,
    turns: Vec<GatewayTurnSummaryJson>,
}

pub(crate) fn sign_turn_attachments_for_response(
    attachments: Option<Value>,
) -> Option<Vec<session_upload::SessionUploadedAttachment>> {
    let atts = attachments?;
    let arr = atts.as_array()?;
    let oss = crate::oss_object_store::OssConfig::from_env();
    let now = chrono::Utc::now();
    let ttl = oss.signed_url_ttl_secs;
    let out: Vec<_> = arr
        .iter()
        .filter_map(|item| {
            let mut att = session_upload::SessionUploadedAttachment::from_json_value(item)?;
            if oss.enabled() {
                if let Some(key) = att
                    .oss_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    if let Ok(url) = oss.presign_get(key, ttl, now) {
                        att.oss_signed_url = Some(url);
                    }
                }
            }
            Some(att)
        })
        .collect();
    Some(out)
}

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/turns",
    tag = "Sessions",
    operation_id = "list_session_turns",
    params(
        ("session_id" = String, Path, description = "Gateway session id"),
        ListSessionTurnsQuery
    ),
    responses(
        (status = 200, description = "Turn summaries for session", body = ListSessionTurnsResponse),
        (status = 404, description = "Session not found")
    )
)]
pub(crate) async fn list_session_turns(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<ListSessionTurnsQuery>,
) -> Result<Json<ListSessionTurnsResponse>, ApiError> {
    if query.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let exists = state
        .session_db
        .session_exists(&session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if !exists {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("session not found: {session_id} projId={}", query.proj_id),
        ));
    }
    let rows = state
        .session_db
        .list_turns_for_session(&session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let worker_profile = state
        .session_db
        .get_worker_profile_json(query.proj_id)
        .await
        .ok()
        .map(|j| pool::profile_mode_label(&j).to_string());
    Ok(Json(ListSessionTurnsResponse {
        session_id,
        proj_id: query.proj_id,
        turns: rows
            .into_iter()
            .map(|r| {
                let attachments = sign_turn_attachments_for_response(r.attachments);
                GatewayTurnSummaryJson {
                    turn_id: r.turn_id,
                    user_prompt: r.user_prompt,
                    status: r.status,
                    created_at_ms: r.created_at_ms,
                    finished_at_ms: r.finished_at_ms,
                    has_report: r.has_report,
                    report_body: r.report_body,
                    failure_detail: r.failure_detail,
                    client_origin: r.client_origin,
                    feedback: r.feedback,
                    extra_session: r.extra_session,
                    attachments,
                    pool_id: r.pool_id,
                    worker_name: r.worker_name,
                    worker_profile: worker_profile.clone(),
                    worker_exec_user: r.worker_exec_user,
                    gateway_id: r.gateway_id,
                    gateway_base: r.gateway_base,
                }
            })
            .collect(),
    }))
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TurnToolsQuery {
    #[serde(rename = "proj_id")]
    #[param(rename = "proj_id")]
    proj_id: i64,
}

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/turns/{turn_id}/tools",
    tag = "Sessions",
    operation_id = "get_turn_tools",
    params(
        ("session_id" = String, Path, description = "Gateway session id"),
        ("turn_id" = String, Path, description = "Turn id (T_<32 hex>)"),
        TurnToolsQuery
    ),
    responses(
        (status = 200, description = "Tool executions for turn", body = turn_tools_api::TurnToolsResponse),
        (status = 404, description = "Turn or session not found")
    )
)]
pub(crate) async fn get_turn_tools(
    State(state): State<AppState>,
    AxumPath((session_id, turn_id)): AxumPath<(String, String)>,
    Query(query): Query<TurnToolsQuery>,
) -> Result<Json<turn_tools_api::TurnToolsResponse>, ApiError> {
    if query.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "proj_id must be >= 1",
        ));
    }
    if !turn_id::validate_turn_id(&turn_id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "turnId must match T_<32 lowercase hex>",
        ));
    }
    let ctx = state
        .session_db
        .get_turn_tools_context(&turn_id, &session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!(
                    "turn or session not found: {turn_id} session={session_id} proj_id={}",
                    query.proj_id
                ),
            )
        })?;
    session_merge::validate_session_home_rel(&ctx.session_home_rel)
        .map_err(session_routing_error)?;
    let user_turn_index = ctx.user_turn_index;
    if user_turn_index < 1 {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid user_turn_index",
        ));
    }
    let turn_status = state
        .session_db
        .get_turn_status(&turn_id, &session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .unwrap_or_else(|| "unknown".to_string());
    if let Ok(pool) = state
        .pool_clients
        .pool_for_turn(&state.session_db, &turn_id, &session_id, query.proj_id)
        .await
    {
        pool_consumer_resolve::maybe_sync_running_turn_progress_from_worker(
            &pool,
            &turn_id,
            &turn_status,
        )
        .await;
    }
    let progress_snap =
        pool_consumer_resolve::resolve_turn_progress(&state.session_db, &turn_id, 500)
            .await
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let session_jsonl = state
        .session_db
        .render_session_jsonl(&session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let tools = turn_tools_api::list_turn_tools_for_session(
        &session_jsonl,
        &progress_snap.events,
        usize::try_from(user_turn_index).unwrap_or(usize::MAX),
        Some(ctx.created_at_ms),
        ctx.finished_at_ms,
    )
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(turn_tools_api::TurnToolsResponse {
        session_id,
        turn_id,
        proj_id: query.proj_id,
        user_turn_index,
        tools,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/turns/{turn_id}/timeline",
    tag = "Sessions",
    operation_id = "get_turn_timeline",
    params(
        ("session_id" = String, Path, description = "Gateway session id"),
        ("turn_id" = String, Path, description = "Turn id (T_<32 hex>)"),
        TurnToolsQuery
    ),
    responses(
        (status = 200, description = "Multi-agent timeline for turn", body = turn_timeline_api::TurnTimelineResponse),
        (status = 404, description = "Turn or session not found")
    )
)]
pub(crate) async fn get_turn_timeline(
    State(state): State<AppState>,
    AxumPath((session_id, turn_id)): AxumPath<(String, String)>,
    Query(query): Query<TurnToolsQuery>,
) -> Result<Json<turn_timeline_api::TurnTimelineResponse>, ApiError> {
    if query.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "proj_id must be >= 1",
        ));
    }
    if !turn_id::validate_turn_id(&turn_id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "turnId must match T_<32 lowercase hex>",
        ));
    }
    let ctx = state
        .session_db
        .get_turn_tools_context(&turn_id, &session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!(
                    "turn or session not found: {turn_id} session={session_id} proj_id={}",
                    query.proj_id
                ),
            )
        })?;
    let turn_status = state
        .session_db
        .get_turn_status(&turn_id, &session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .unwrap_or_else(|| "unknown".to_string());
    if let Ok(pool) = state
        .pool_clients
        .pool_for_turn(&state.session_db, &turn_id, &session_id, query.proj_id)
        .await
    {
        pool_consumer_resolve::maybe_sync_running_turn_progress_from_worker(
            &pool,
            &turn_id,
            &turn_status,
        )
        .await;
    }
    let timeline = pool_consumer_resolve::resolve_turn_timeline(
        &state.session_db,
        &turn_id,
        ctx.created_at_ms,
        ctx.finished_at_ms,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    Ok(Json(turn_timeline_api::TurnTimelineResponse {
        session_id,
        turn_id,
        proj_id: query.proj_id,
        task_created_at_ms: Some(ctx.created_at_ms),
        task_finished_at_ms: ctx.finished_at_ms,
        timeline,
    }))
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnCancelResponse {
    #[serde(rename = "sessionId", alias = "session_id")]
    session_id: String,
    #[serde(rename = "turnId", alias = "turn_id")]
    turn_id: String,
    #[serde(rename = "projId", alias = "proj_id")]
    proj_id: i64,
    status: String,
    #[serde(rename = "cancelApplied")]
    cancel_applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    error: Option<Value>,
}

pub(crate) fn turn_cancel_idempotent_error(status: &str) -> Value {
    let detail = match status {
        "cancelled" => "turn already cancelled; duplicate cancel ignored".to_string(),
        "succeeded" => "turn already succeeded; cancel had no effect".to_string(),
        "failed" => "turn already failed; cancel had no effect".to_string(),
        other => format!("turn already in terminal state ({other}); cancel had no effect"),
    };
    json!({
        "detail": detail,
        "outcome": "idempotent",
        "cancelApplied": false,
        "statusAtCancel": status,
    })
}

pub(crate) async fn cancel_session_turn_cold(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    proj_id: i64,
) -> Result<TurnCancelResponse, ApiError> {
    let Some(status) = state
        .session_db
        .get_turn_status(turn_id, session_id, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("turn or session not found: {turn_id} session={session_id} proj_id={proj_id}"),
        ));
    };
    if task_status_is_terminal_for_cancel(&status) {
        let status_at_cancel = status.clone();
        return Ok(TurnCancelResponse {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            proj_id,
            status,
            cancel_applied: false,
            error: Some(turn_cancel_idempotent_error(&status_at_cancel)),
        });
    }
    finalize_solve_turn_cancelled(&state.session_db, turn_id).await;
    Ok(TurnCancelResponse {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        proj_id,
        status: "cancelled".to_string(),
        cancel_applied: true,
        error: Some(json!({
            "detail": "cancelled by client",
            "outcome": "cancelled",
            "cancelApplied": true,
        })),
    })
}

pub(crate) enum TurnMemoryCancel {
    Idempotent(TurnCancelResponse),
    Applied(Option<AbortHandle>),
}

pub(crate) async fn abort_memory_session_task_if_active(state: &AppState, session_id: &str, proj_id: i64) {
    let (cancel_handle, turn_id) = {
        let mut tasks = state.tasks.lock().await;
        let Some(inner) = tasks.get_mut(session_id) else {
            return;
        };
        if inner.proj_id != proj_id || task_status_is_terminal_for_cancel(&inner.record.status) {
            return;
        }
        let turn_id = inner.record.turn_id.clone();
        let h = inner.cancel.take();
        inner.record.status = "cancelled".to_string();
        inner.record.finished_at_ms = Some(now_ms());
        inner.record.result = None;
        inner.record.error = Some(json!({
            "detail": "cancelled by client (session slot released after turn cancel)",
            "outcome": "cancelled",
            "cancelApplied": true,
        }));
        tasks.remove(session_id);
        (h, turn_id)
    };
    if let Some((pool, idx)) = state.docker_slots.lock().await.remove(session_id) {
        let _ = pool.force_kill_slot(idx).await;
    }
    if let Some(h) = cancel_handle {
        h.abort();
    }
    finalize_solve_turn_cancelled(&state.session_db, &turn_id).await;
}

pub(crate) async fn try_memory_cancel_turn(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    proj_id: i64,
) -> Result<Option<TurnMemoryCancel>, ApiError> {
    let mut tasks = state.tasks.lock().await;
    let Some(inner) = tasks.get_mut(session_id) else {
        return Ok(None);
    };
    if inner.record.turn_id != turn_id {
        return Ok(None);
    }
    if inner.proj_id != proj_id {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!(
                "turn proj_id mismatch: turn={turn_id} session={session_id} expected proj_id={proj_id}"
            ),
        ));
    }
    if task_status_is_terminal_for_cancel(&inner.record.status) {
        let status = inner.record.status.clone();
        return Ok(Some(TurnMemoryCancel::Idempotent(TurnCancelResponse {
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            proj_id,
            status,
            cancel_applied: false,
            error: Some(turn_cancel_idempotent_error(inner.record.status.as_str())),
        })));
    }
    let h = inner.cancel.take();
    inner.record.status = "cancelled".to_string();
    inner.record.finished_at_ms = Some(now_ms());
    inner.record.result = None;
    inner.record.error = Some(json!({
        "detail": "cancelled by client",
        "outcome": "cancelled",
        "cancelApplied": true,
    }));
    Ok(Some(TurnMemoryCancel::Applied(h)))
}

#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/turns/{turn_id}/cancel",
    tag = "Sessions",
    operation_id = "cancel_session_turn",
    params(
        ("session_id" = String, Path, description = "Gateway session id"),
        ("turn_id" = String, Path, description = "Turn id (T_<32 hex>)"),
        TurnToolsQuery
    ),
    responses(
        (status = 200, description = "Turn cancel result", body = TurnCancelResponse),
        (status = 404, description = "Turn or session not found")
    )
)]
pub(crate) async fn cancel_session_turn(
    State(state): State<AppState>,
    AxumPath((session_id, turn_id)): AxumPath<(String, String)>,
    Query(query): Query<TurnToolsQuery>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<TurnCancelResponse>, ApiError> {
    if query.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "proj_id must be >= 1",
        ));
    }
    if !turn_id::validate_turn_id(&turn_id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "turnId must match T_<32 lowercase hex>",
        ));
    }

    // If this turn is owned by another online gateway and we have no local memory task,
    // reverse-proxy cancel to the owner. Author: kejiqing
    let local_memory = {
        let tasks = state.tasks.lock().await;
        tasks
            .get(&session_id)
            .is_some_and(|inner| inner.record.turn_id == turn_id)
    };
    if !local_memory {
        if let Ok(Some(owner_base)) =
            crate::gateway_owner_proxy::resolve_turn_owner_proxy_base(
                &state.session_db,
                state.gateway_identity.as_ref(),
                &turn_id,
                &session_id,
                query.proj_id,
            )
            .await
        {
            let path = format!(
                "/v1/sessions/{session_id}/turns/{turn_id}/cancel?projId={}",
                query.proj_id
            );
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let url = format!("{}{}", owner_base.trim_end_matches('/'), path);
            let upstream = client
                .post(&url)
                .send()
                .await
                .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
            let status = upstream.status();
            let bytes = upstream
                .bytes()
                .await
                .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.to_string()))?;
            if !status.is_success() {
                return Err(ApiError::new(
                    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
                    String::from_utf8_lossy(&bytes).to_string(),
                ));
            }
            let parsed: TurnCancelResponse = serde_json::from_slice(&bytes).map_err(|e| {
                ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    format!("parse cancel proxy json: {e}"),
                )
            })?;
            return Ok(Json(parsed));
        }
    }

    match try_memory_cancel_turn(&state, &session_id, &turn_id, query.proj_id).await? {
        Some(TurnMemoryCancel::Idempotent(out)) => {
            info!(
                request_id = %http_request_id.0,
                session_id = %session_id,
                turn_id = %turn_id,
                endpoint = "/v1/sessions/{session_id}/turns/{turn_id}/cancel",
                phase = "cancel_idempotent",
                "gateway_turn_cancel"
            );
            return Ok(Json(out));
        }
        Some(TurnMemoryCancel::Applied(cancel_handle)) => {
            if let Some((pool, idx)) = state.docker_slots.lock().await.remove(&session_id) {
                let _ = pool.force_kill_slot(idx).await;
            }
            if let Some(h) = cancel_handle {
                h.abort();
            }
            {
                let mut tasks = state.tasks.lock().await;
                tasks.remove(&session_id);
            }
            finalize_solve_turn_cancelled(&state.session_db, &turn_id).await;
            info!(
                request_id = %http_request_id.0,
                session_id = %session_id,
                turn_id = %turn_id,
                endpoint = "/v1/sessions/{session_id}/turns/{turn_id}/cancel",
                phase = "cancel_memory",
                "gateway_turn_cancel"
            );
            return Ok(Json(TurnCancelResponse {
                session_id,
                turn_id,
                proj_id: query.proj_id,
                status: "cancelled".to_string(),
                cancel_applied: true,
                error: Some(json!({
                    "detail": "cancelled by client",
                    "outcome": "cancelled",
                    "cancelApplied": true,
                })),
            }));
        }
        None => {}
    }

    let out = cancel_session_turn_cold(&state, &session_id, &turn_id, query.proj_id).await?;
    if out.cancel_applied {
        abort_memory_session_task_if_active(&state, &session_id, query.proj_id).await;
    }
    info!(
        request_id = %http_request_id.0,
        session_id = %out.session_id,
        turn_id = %out.turn_id,
        cancel_applied = out.cancel_applied,
        endpoint = "/v1/sessions/{session_id}/turns/{turn_id}/cancel",
        phase = "cancel_cold",
        "gateway_turn_cancel"
    );
    Ok(Json(out))
}

