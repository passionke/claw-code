// Fragment of routes::app (include!). Author: kejiqing






#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ProbeQuery {
    #[serde(rename = "probe_timeout_seconds")]
    #[param(rename = "probe_timeout_seconds")]
    probe_timeout_seconds: Option<u64>,
}

#[utoipa::path(
    post,
    path = "/v1/solve",
    tag = "Solve",
    operation_id = "solve",
    request_body(
        content = SolveRequest,
        description = "Text-only or multimodal (attachments from POST /v1/sessions/{id}/files). Image/video/audio require matching model capability flags (supportsVision/supportsVideo/supportsAudio).",
        examples(
            ("TextOnly" = (
                summary = "Text-only solve",
                value = json!({
                    "projId": 1,
                    "userPrompt": "connectivity check"
                })
            )),
            ("MultimodalImage" = (
                summary = "Multimodal image (upload files first; model must supportsVision)",
                value = json!({
                    "projId": 1,
                    "userPrompt": "请描述这张图",
                    "attachments": [{
                        "path": "uploads/photo.png",
                        "mime": "image/png",
                        "kind": "image",
                        "name": "photo.png",
                        "size": 12345
                    }]
                })
            )),
            ("MultimodalVideo" = (
                summary = "Multimodal video (prefer OSS url; model must supportsVideo)",
                value = json!({
                    "projId": 1,
                    "userPrompt": "请总结这段视频",
                    "attachments": [{
                        "path": "uploads/clip.mp4",
                        "mime": "video/mp4",
                        "kind": "video",
                        "name": "clip.mp4",
                        "size": 1_234_567,
                        "url": "https://example.oss-cn-hangzhou.aliyuncs.com/sessions/.../clip.mp4?Expires=..."
                    }]
                })
            )),
            ("MultimodalAudio" = (
                summary = "Multimodal audio (prefer OSS url; model must supportsAudio)",
                value = json!({
                    "projId": 1,
                    "userPrompt": "请转写这段音频",
                    "attachments": [{
                        "path": "uploads/voice.wav",
                        "mime": "audio/wav",
                        "kind": "audio",
                        "name": "voice.wav",
                        "size": 234_567,
                        "url": "https://example.oss-cn-hangzhou.aliyuncs.com/sessions/.../voice.wav?Expires=..."
                    }]
                })
            ))
        )
    ),
    responses(
        (status = 200, description = "Solve finished", body = SolveResponse),
        (status = 400, description = "Invalid request or unknown sessionId"),
        (status = 409, description = "Session enqueue blocked")
    )
)]
pub(crate) async fn solve(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(http_request_id): Extension<HttpRequestId>,
    Extension(id_kind): Extension<session_merge::HttpRequestIdKind>,
    Json(req): Json<SolveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let body_sid = session_merge::trim_session_id(req.session_id.as_deref());
    let effective =
        session_merge::merge_effective_session_id(body_sid, &http_request_id.0, id_kind)
            .map_err(session_routing_error)?;
    info!(
        request_id = %effective,
        proj_id = req.proj_id,
        endpoint = "/v1/solve",
        phase = "accepted",
        "gateway_solve"
    );
    let client_origin = resolve_request_client_origin(req.extra_session.as_ref(), &headers);
    validate_solve_request(&state.session_db, &req).await?;
    state
        .session_db
        .assert_session_can_enqueue(&effective, req.proj_id)
        .await
        .map_err(|reason| {
            ApiError::new(
                StatusCode::CONFLICT,
                format!("session enqueue blocked: {reason}"),
            )
        })?;
    state
        .pool_clients
        .assert_proj_worker_profile_supported(&state.session_db, req.proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    let new_turn_id = turn_id::mint_turn_id();
    let (_, prebind_pool_id) = state
        .pool_clients
        .pool_and_id_for_proj(&state.session_db, req.proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    register_solve_turn(
        &state.session_db,
        &new_turn_id,
        &effective,
        &req,
        Some(prebind_pool_id.as_str()),
        client_origin.as_deref(),
        Some(state.gateway_identity.as_ref()),
    )
    .await?;
    let result = run_solve_request(
        state.clone(),
        req,
        RunSolveContext {
            request_id: effective.clone(),
            task_id: None,
            turn_id: new_turn_id.clone(),
            skip_session_db: false,
            client_origin,
        },
    )
    .await;
    match &result {
        Ok(success) => {
            finalize_solve_turn_success(Arc::clone(&state.session_db), &new_turn_id, success).await;
        }
        Err(err) => {
            finalize_solve_turn_failed(&state.session_db, &new_turn_id, err).await;
        }
    }
    let result = result?;
    let claw = HeaderValue::from_str(&effective).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid characters in session id for response header",
        )
    })?;
    let xrid = header::HeaderName::from_static("x-request-id");
    let csid = header::HeaderName::from_static("claw-session-id");
    Ok((
        AppendHeaders([(xrid, claw.clone()), (csid, claw)]),
        Json(result),
    ))
}

pub(crate) fn solve_async_response_headers(
    effective: &str,
) -> Result<AppendHeaders<[(header::HeaderName, HeaderValue); 2]>, ApiError> {
    let claw = HeaderValue::from_str(effective).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid characters in session id for response header",
        )
    })?;
    let xrid = header::HeaderName::from_static("x-request-id");
    let csid = header::HeaderName::from_static("claw-session-id");
    Ok(AppendHeaders([(xrid, claw.clone()), (csid, claw)]))
}

pub(crate) async fn enqueue_solve_async(
    state: AppState,
    http_request_id: HttpRequestId,
    id_kind: session_merge::HttpRequestIdKind,
    req: SolveRequest,
    endpoint: &'static str,
    client_origin: Option<String>,
) -> Result<SolveAsyncResponse, ApiError> {
    enqueue_solve_async_with_turn(
        state,
        http_request_id,
        id_kind,
        req,
        endpoint,
        client_origin,
        None,
    )
    .await
}

pub(crate) async fn enqueue_solve_async_with_turn(
    state: AppState,
    http_request_id: HttpRequestId,
    id_kind: session_merge::HttpRequestIdKind,
    req: SolveRequest,
    endpoint: &'static str,
    client_origin: Option<String>,
    preassigned_turn_id: Option<String>,
) -> Result<SolveAsyncResponse, ApiError> {
    let body_sid = session_merge::trim_session_id(req.session_id.as_deref());
    let effective =
        session_merge::merge_effective_session_id(body_sid, &http_request_id.0, id_kind)
            .map_err(session_routing_error)?;
    if session_merge::trim_session_id(req.session_id.as_deref()).is_some() {
        let row = state
            .session_db
            .get_session_home_rel(&effective, req.proj_id)
            .await
            .map_err(|e| session_db_err(&e))?;
        if row.is_none() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "unknown sessionId (no session history for this projId)",
            ));
        }
    }
    let task_id = effective.clone();
    let proj_id = req.proj_id;
    {
        let tasks = state.tasks.lock().await;
        if let Some(inner) = tasks.get(&task_id) {
            if inner.record.status == "queued" || inner.record.status == "running" {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "session has active async task",
                ));
            }
        }
    }
    validate_solve_request(&state.session_db, &req).await?;
    state
        .session_db
        .assert_session_can_enqueue(&effective, proj_id)
        .await
        .map_err(|reason| {
            ApiError::new(
                StatusCode::CONFLICT,
                format!("session enqueue blocked: {reason}"),
            )
        })?;
    state
        .pool_clients
        .assert_proj_worker_profile_supported(&state.session_db, proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    let new_turn_id = preassigned_turn_id.unwrap_or_else(turn_id::mint_turn_id);
    let (_, prebind_pool_id) = state
        .pool_clients
        .pool_and_id_for_proj(&state.session_db, proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    register_solve_turn(
        &state.session_db,
        &new_turn_id,
        &effective,
        &req,
        Some(prebind_pool_id.as_str()),
        client_origin.as_deref(),
        Some(state.gateway_identity.as_ref()),
    )
    .await?;
    if let Some(rel) = state
        .session_db
        .get_session_home_rel(&effective, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        let home = join_session_home(&state.cfg.work_root, &rel);
        if let Err(e) = reset_task_progress(&home, &effective) {
            warn!(error = %e, "reset task progress before async solve failed");
        }
        let _ = truncate_progress_history(&home);
    }
    info!(
        request_id = %effective,
        task_id = %task_id,
        proj_id = req.proj_id,
        endpoint,
        phase = "queued",
        "gateway_solve_async"
    );
    {
        let mut tasks = state.tasks.lock().await;
        let queue = gateway_queue_snapshot(&tasks);
        let initial_desc = resolve_current_task_desc("queued", &queue, false, None);
        tasks.insert(
            task_id.clone(),
            TaskInner {
                record: TaskRecord {
                    task_id: task_id.clone(),
                    session_id: effective.clone(),
                    request_id: effective.clone(),
                    proj_id,
                    status: "queued".to_string(),
                    created_at_ms: now_ms(),
                    started_at_ms: None,
                    finished_at_ms: None,
                    current_task_desc: initial_desc,
                    progress_updated_at_ms: None,
                    result: None,
                    error: None,
                    turn_id: new_turn_id.clone(),
                    progress_history: Vec::new(),
                    has_report: false,
                    report_time_ms: None,
                    plan_title: None,
                    todos: Vec::new(),
                    interaction_mode: req.interaction_mode.clone(),
                    plan_phase: if gateway_solve_turn::InteractionMode::parse(
                        req.interaction_mode.as_deref(),
                    )
                    .is_plan()
                    {
                        Some("planning".into())
                    } else {
                        None
                    },
                    plan_id: None,
                    plan_markdown: None,
                    plan_turn_id: None,
                    pool_id: Some(prebind_pool_id.clone()),
                    worker_name: None,
                    worker_profile: state
                        .session_db
                        .get_worker_profile_json(proj_id)
                        .await
                        .ok()
                        .map(|j| pool::profile_mode_label(&j).to_string()),
                    worker_exec_user: None,
                    gateway_id: Some(state.gateway_identity.gateway_id.clone()),
                    gateway_base: Some(state.gateway_identity.gateway_base.clone()),
                },
                cancel: None,
                proj_id,
            },
        );
    }
    spawn_task_progress_poller(state.clone(), task_id.clone());
    let state_clone = state.clone();
    let task_id_for_worker = task_id.clone();
    let rid = effective.clone();
    let turn_id_for_worker = new_turn_id.clone();
    let client_origin_for_worker = client_origin.clone();
    let join = tokio::spawn(async move {
        {
            let mut tasks = state_clone.tasks.lock().await;
            if let Some(inner) = tasks.get_mut(&task_id_for_worker) {
                if inner.record.turn_id != turn_id_for_worker {
                    return;
                }
                if inner.record.status == "cancelled" {
                    inner.cancel = None;
                    finalize_solve_turn_cancelled(&state_clone.session_db, &turn_id_for_worker)
                        .await;
                    return;
                }
                inner.record.status = "running".to_string();
                inner.record.started_at_ms = Some(now_ms());
            }
        }
        set_solve_turn_status(
            &state_clone.session_db,
            &turn_id_for_worker,
            "running",
            false,
        )
        .await;
        info!(
            request_id = %rid,
            task_id = %task_id_for_worker,
            turn_id = %turn_id_for_worker,
            phase = "running",
            "gateway_solve_async"
        );
        let result = run_solve_request(
            state_clone.clone(),
            req.clone(),
            RunSolveContext {
                request_id: rid.clone(),
                task_id: Some(task_id_for_worker.clone()),
                turn_id: turn_id_for_worker.clone(),
                skip_session_db: false,
                client_origin: client_origin_for_worker,
            },
        )
        .await;
        let success_for_plan = match &result {
            Ok(v) => Some(v.clone()),
            Err(_) => None,
        };
        let refresh_progress = {
            let mut tasks = state_clone.tasks.lock().await;
            let Some(inner) = tasks.get_mut(&task_id_for_worker) else {
                return;
            };
            if inner.record.turn_id != turn_id_for_worker {
                return;
            }
            inner.cancel = None;
            if inner.record.status == "cancelled" {
                finalize_solve_turn_cancelled(&state_clone.session_db, &turn_id_for_worker).await;
                return;
            }
            inner.record.finished_at_ms = Some(now_ms());
            match result {
                Ok(ref v) => {
                    let duration_ms = v.duration_ms;
                    inner.record.status = "succeeded".to_string();
                    inner.record.result = Some(v.clone());
                    finalize_solve_turn_success(
                        Arc::clone(&state_clone.session_db),
                        &turn_id_for_worker,
                        v,
                    )
                    .await;
                    info!(
                        request_id = %rid,
                        task_id = %task_id_for_worker,
                        phase = "succeeded",
                        duration_ms,
                        "gateway_solve_async"
                    );
                }
                Err(ref e) => {
                    inner.record.status = "failed".to_string();
                    inner.record.error =
                        Some(json!({"status_code": e.status.as_u16(), "detail": e.message}));
                    finalize_solve_turn_failed(&state_clone.session_db, &turn_id_for_worker, e)
                        .await;
                    warn!(
                        request_id = %rid,
                        task_id = %task_id_for_worker,
                        phase = "failed",
                        status_code = e.status.as_u16(),
                        error = %e.message,
                        "gateway_solve_async"
                    );
                }
            }
            true
        };
        if let Some(ref v) = success_for_plan {
            maybe_persist_plan_after_solve(&state_clone, &req, &rid, &turn_id_for_worker, v).await;
            let mut tasks = state_clone.tasks.lock().await;
            if let Some(inner) = tasks.get_mut(&task_id_for_worker) {
                enrich_task_record_with_plan(&state_clone, &mut inner.record).await;
            }
        }
        if refresh_progress {
            refresh_task_progress(&state_clone, &task_id_for_worker).await;
        }
    });
    let cancel = join.abort_handle();
    {
        let mut tasks = state.tasks.lock().await;
        if let Some(inner) = tasks.get_mut(&task_id) {
            inner.cancel = Some(cancel);
        }
    }
    refresh_task_progress(&state, &task_id).await;
    Ok(SolveAsyncResponse {
        task_id: task_id.clone(),
        session_id: effective.clone(),
        request_id: effective.clone(),
        turn_id: new_turn_id.clone(),
        status: "queued".to_string(),
        poll_url: format!("/v1/tasks/{task_id}"),
        pool_id: state
            .session_db
            .get_turn_pool_id(&new_turn_id, &effective, proj_id)
            .await
            .ok()
            .flatten()
            .or_else(|| state.cfg.co_located_pool_id.clone()),
        worker_name: None,
        worker_profile: state
            .session_db
            .get_worker_profile_json(proj_id)
            .await
            .ok()
            .map(|j| pool::profile_mode_label(&j).to_string()),
        worker_exec_user: None,
        gateway_id: Some(state.gateway_identity.gateway_id.clone()),
        gateway_base: Some(state.gateway_identity.gateway_base.clone()),
    })
}

#[utoipa::path(
    post,
    path = "/v1/start",
    tag = "Solve",
    operation_id = "solve_start",
    summary = "Register gateway session (sync)",
    description = "Synchronously writes (sessionId, dsId) to gateway SQLite, prepares session workspace, and returns sessionId/requestId. Does not run solve.",
    request_body = StartRequest,
    responses(
        (status = 200, description = "Session registered", body = SolveStartResponse),
        (status = 400, description = "Unknown sessionId for continuation")
    )
)]
pub(crate) async fn solve_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(http_request_id): Extension<HttpRequestId>,
    Extension(id_kind): Extension<session_merge::HttpRequestIdKind>,
    Json(req): Json<StartRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let body_sid = session_merge::trim_session_id(req.session_id.as_deref());
    let effective =
        session_merge::merge_effective_session_id(body_sid, &http_request_id.0, id_kind)
            .map_err(session_routing_error)?;
    if body_sid.is_some() {
        let row = state
            .session_db
            .get_session_home_rel(&effective, req.proj_id)
            .await
            .map_err(|e| session_db_err(&e))?;
        if row.is_none() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "unknown sessionId (no session history for this projId)",
            ));
        }
    }
    let client_origin = client_origin::resolve_client_origin(
        req.extra_session.as_ref(),
        client_origin_from_headers(&headers),
    );
    prepare_gateway_session(
        &state,
        req.proj_id,
        req.session_id.as_deref(),
        req.extra_session.as_ref(),
        &effective,
        false,
        client_origin.as_deref(),
    )
    .await?;
    info!(
        request_id = %effective,
        proj_id = req.proj_id,
        endpoint = "/v1/start",
        phase = "session_ready",
        "gateway_start: session registered in SQLite before response"
    );
    let headers = solve_async_response_headers(&effective)?;
    Ok((
        headers,
        Json(SolveStartResponse {
            session_id: effective.clone(),
            request_id: effective,
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/v1/solve_async",
    tag = "Solve",
    operation_id = "solve_async",
    request_body(
        content = SolveRequest,
        description = "Text-only or multimodal (attachments from POST /v1/sessions/{id}/files). Image/video/audio require matching model capability flags (supportsVision/supportsVideo/supportsAudio).",
        examples(
            ("TextOnly" = (
                summary = "Text-only async solve",
                value = json!({
                    "projId": 1,
                    "userPrompt": "connectivity check"
                })
            )),
            ("MultimodalImage" = (
                summary = "Multimodal image (upload files first; model must supportsVision)",
                value = json!({
                    "projId": 1,
                    "userPrompt": "请描述这张图",
                    "attachments": [{
                        "path": "uploads/photo.png",
                        "mime": "image/png",
                        "kind": "image",
                        "name": "photo.png",
                        "size": 12345
                    }]
                })
            )),
            ("MultimodalVideo" = (
                summary = "Multimodal video (prefer OSS url; model must supportsVideo)",
                value = json!({
                    "projId": 1,
                    "userPrompt": "请总结这段视频",
                    "attachments": [{
                        "path": "uploads/clip.mp4",
                        "mime": "video/mp4",
                        "kind": "video",
                        "name": "clip.mp4",
                        "size": 1_234_567,
                        "url": "https://example.oss-cn-hangzhou.aliyuncs.com/sessions/.../clip.mp4?Expires=..."
                    }]
                })
            )),
            ("MultimodalAudio" = (
                summary = "Multimodal audio (prefer OSS url; model must supportsAudio)",
                value = json!({
                    "projId": 1,
                    "userPrompt": "请转写这段音频",
                    "attachments": [{
                        "path": "uploads/voice.wav",
                        "mime": "audio/wav",
                        "kind": "audio",
                        "name": "voice.wav",
                        "size": 234_567,
                        "url": "https://example.oss-cn-hangzhou.aliyuncs.com/sessions/.../voice.wav?Expires=..."
                    }]
                })
            ))
        )
    ),
    responses(
        (status = 200, description = "Async task created", body = SolveAsyncResponse),
        (status = 400, description = "Unknown sessionId for continuation"),
        (status = 409, description = "Session has active async task or enqueue blocked")
    )
)]
pub(crate) async fn solve_async(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(http_request_id): Extension<HttpRequestId>,
    Extension(id_kind): Extension<session_merge::HttpRequestIdKind>,
    Json(req): Json<SolveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let client_origin = resolve_request_client_origin(req.extra_session.as_ref(), &headers);
    let out = enqueue_solve_async(
        state,
        http_request_id,
        id_kind,
        req,
        "/v1/solve_async",
        client_origin,
    )
    .await?;
    let headers = solve_async_response_headers(&out.session_id)?;
    Ok((headers, Json(out)))
}

pub(crate) async fn validate_solve_extra_session_for_ds(
    db: &session_db::GatewaySessionDb,
    proj_id: i64,
    extra_session: Option<&Value>,
) -> Result<(), ApiError> {
    validate_extra_session(extra_session)?;
    let fields_json = match db.get_project_config(proj_id).await {
        Ok(Some(row)) => row.extra_session_fields_json,
        Ok(None) => json!([]),
        Err(e) => return Err(session_db_err(&e)),
    };
    let fields = project_extra_session::parse_extra_session_fields_json(&fields_json)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    project_extra_session::validate_extra_session_against_fields(extra_session, &fields)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(())
}

pub(crate) async fn validate_solve_request(
    db: &session_db::GatewaySessionDb,
    req: &SolveRequest,
) -> Result<(), ApiError> {
    if req.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    if req.max_iterations == Some(0) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "maxIterations must be >= 1",
        ));
    }
    let has_attachments = req.attachments.as_ref().is_some_and(|a| !a.is_empty());
    if req.user_prompt.trim().is_empty() && !has_attachments {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "userPrompt cannot be empty",
        ));
    }
    if let Some(atts) = req.attachments.as_ref() {
        let needs_vision = atts
            .iter()
            .any(|a| a.kind == gateway_solve_turn::SolveAttachmentKind::Image);
        let needs_video = atts
            .iter()
            .any(|a| a.kind == gateway_solve_turn::SolveAttachmentKind::Video);
        let needs_audio = atts
            .iter()
            .any(|a| a.kind == gateway_solve_turn::SolveAttachmentKind::Audio);
        if needs_vision || needs_video || needs_audio {
            let runtime = gateway_project_llm::load_effective_llm_runtime(db, req.proj_id)
                .await
                .map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("load effective LLM failed: {e}"),
                    )
                })?;
            if needs_vision && !runtime.as_ref().is_some_and(|r| r.supports_vision) {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "MODEL_NO_VISION: current model does not support images; switch model or remove image attachments",
                ));
            }
            if needs_video && !runtime.as_ref().is_some_and(|r| r.supports_video) {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "MODEL_NO_VIDEO: current model does not support video; switch model or remove video attachments",
                ));
            }
            if needs_audio && !runtime.as_ref().is_some_and(|r| r.supports_audio) {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "MODEL_NO_AUDIO: current model does not support audio; switch model or remove audio attachments",
                ));
            }
        }
    }
    validate_solve_extra_session_for_ds(db, req.proj_id, req.extra_session.as_ref()).await?;
    let mode = gateway_solve_turn::InteractionMode::parse(req.interaction_mode.as_deref());
    if mode.is_plan() {
        let profile = db
            .get_worker_profile_json(req.proj_id)
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("load worker profile failed: {e}"),
                )
            })?;
        let relaxed = crate::pool::effective_mode(
            crate::pool::relaxed_worker_allowed_from_env(),
            &profile,
        ) == crate::pool::WorkerProfileMode::Relaxed;
        if !relaxed {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "interactionMode=plan requires relaxed worker profile",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_extra_session(extra_session: Option<&Value>) -> Result<(), ApiError> {
    if let Some(extra_session) = extra_session {
        if !extra_session.is_object() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "extraSession must be a JSON object when present",
            ));
        }
        if let Ok(serialized) = serde_json::to_vec(extra_session) {
            if serialized.len() > 8 * 1024 {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "extraSession is too large (max 8KB)",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) async fn prepare_gateway_session(
    state: &AppState,
    proj_id: i64,
    body_session_id: Option<&str>,
    extra_session: Option<&Value>,
    request_id: &str,
    skip_session_db: bool,
    client_origin: Option<&str>,
) -> Result<PreparedGatewaySession, ApiError> {
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    validate_extra_session(extra_session)?;
    validate_proj_exists(proj_id, &state.cfg.ds_registry_path).await?;

    let _session_lock_guard: Option<OwnedMutexGuard<()>> = if skip_session_db {
        None
    } else {
        Some(
            get_session_solve_lock(state, proj_id, request_id)
                .await
                .lock_owned()
                .await,
        )
    };

    let proj_base = pool::gateway_proj_work_dir(&state.cfg.work_root, proj_id)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let explicit_continuation = session_merge::trim_session_id(body_session_id).is_some();

    let (session_home, need_insert_row, purge_mcp_discovery, session_fs_label) = if skip_session_db
    {
        let session_fs_id = session_merge::sessions_directory_segment(request_id);
        let session_home = proj_base.join("sessions").join(&session_fs_id);
        (session_home, false, true, session_fs_id)
    } else {
        let row_opt = state
            .session_db
            .get_session_home_rel(request_id, proj_id)
            .await
            .map_err(|e| session_db_err(&e))?;
        if let Some(rel) = row_opt {
            session_merge::validate_session_home_rel(&rel).map_err(session_routing_error)?;
            let session_home =
                session_merge::join_session_home_from_rel(&state.cfg.work_root, &rel);
            let exists = fs::metadata(&session_home).await.is_ok_and(|m| m.is_dir());
            if exists {
                (session_home, false, false, rel)
            } else {
                // ② Gateway cache is optional; PG is SoT — recreate local session tree. Author: kejiqing
                (session_home, false, true, rel)
            }
        } else if explicit_continuation {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "unknown sessionId (no session history for this projId)",
            ));
        } else {
            let session_fs_id = session_merge::sessions_directory_segment(request_id);
            let session_home = proj_base.join("sessions").join(&session_fs_id);
            (session_home, true, true, session_fs_id)
        }
    };

    let session_home_rel =
        session_merge::session_home_rel_under_work_root(&state.cfg.work_root, &session_home)
            .map_err(session_routing_error)?;

    fs::create_dir_all(session_home.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create session work dir failed: {e}"),
            )
        })?;

    {
        let proj_lock = get_proj_lock(state, proj_id).await;
        let _guard = proj_lock.lock().await;
        ensure_proj_ready(state, proj_id).await?;
        fs::create_dir_all(&proj_base).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create ds dir failed: {e}"),
            )
        })?;
        ensure_workspace_initialized(&state.cfg.claw_bin, &proj_base).await?;
        let settings = build_settings(state, proj_id).await;
        let settings_content = serde_json::to_vec_pretty(&settings).map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize settings failed: {e}"),
            )
        })?;
        fs::write(session_home.join(".claw/settings.json"), &settings_content)
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write settings failed: {e}"),
                )
            })?;
        if purge_mcp_discovery {
            let _ = fs::remove_file(session_home.join(".claw/mcp_discovery_cache.json")).await;
        }
    }

    // Optional gateway-local cache (②): uid-align session dir; pool v1 does not bind it. kejiqing
    let pool_bin = container_runtime_bin();
    pool::ensure_session_tree_owned_for_worker_with_runtime_fallback(&pool_bin, &session_home)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session workspace ownership for pool worker failed: {e}"),
            )
        })?;

    if need_insert_row {
        state
            .session_db
            .insert_session(
                request_id,
                proj_id,
                &session_home_rel,
                now_ms(),
                client_origin,
            )
            .await
            .map_err(|e| session_db_err(&e))?;
    } else if !skip_session_db {
        state
            .session_db
            .touch_updated(request_id, proj_id, now_ms())
            .await
            .map_err(|e| session_db_err(&e))?;
    }

    Ok(PreparedGatewaySession {
        session_home,
        session_home_rel,
        session_fs_label,
    })
}

pub(crate) async fn run_solve_request(
    state: AppState,
    req: SolveRequest,
    ctx: RunSolveContext,
) -> Result<SolveResponse, ApiError> {
    if !ctx.skip_session_db {
        set_solve_turn_status(&state.session_db, &ctx.turn_id, "running", false).await;
    }
    let started = Instant::now();
    let timeout_seconds = req
        .timeout_seconds
        .unwrap_or(state.cfg.default_timeout_seconds);
    info!(
        target: "claw_gateway_orchestration",
        component = "solve",
        request_id = %ctx.request_id,
        task_id = ctx.task_id.as_deref().unwrap_or("-"),
        proj_id = req.proj_id,
        phase = "solve_run_start",
        timeout_seconds,
        "gateway_solve accepted; validating and preparing workspace"
    );
    let project_selected = project_selected_allowed_tools(&state, req.proj_id).await?;
    let mut effective_allowed_tools = resolve_effective_allowed_tools_for_ds(
        project_selected.as_deref(),
        req.allowed_tools.as_deref(),
    )?;
    ensure_report_progress_in_allowed_tools(&mut effective_allowed_tools);

    let prepared = prepare_gateway_session(
        &state,
        req.proj_id,
        req.session_id.as_deref(),
        req.extra_session.as_ref(),
        &ctx.request_id,
        ctx.skip_session_db,
        ctx.client_origin.as_deref(),
    )
    .await?;

    info!(
        target: "claw_gateway_orchestration",
        component = "solve_prepare",
        phase = "workspace_ready",
        proj_id = req.proj_id,
        request_id = %ctx.request_id,
        task_id = ctx.task_id.as_deref(),
        session_fs_id = %prepared.session_fs_label,
        session_home = %prepared.session_home.display(),
        solve_backend = "e2b",
        timeout_seconds,
        "session .claw/settings.json written; starting solve (e2b sandbox)"
    );

    let (pool, pool_id) = state
        .pool_clients
        .pool_and_id_for_proj(&state.session_db, req.proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    solve_pool::run_solve_request_docker(
        state,
        req,
        ctx,
        pool,
        &pool_id,
        started,
        effective_allowed_tools,
        solve_pool::SolveSessionPaths {
            session_home: prepared.session_home,
            session_home_rel: prepared.session_home_rel,
        },
    )
    .await
}

pub(crate) async fn apply_settings_and_probe(
    state: &AppState,
    proj_id: i64,
    probe_timeout_seconds: u64,
) -> Result<(Value, Vec<String>, i64, String, Vec<String>), ApiError> {
    let work_dir = state.cfg.work_root.join(format!("proj_{proj_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let settings = {
        let lock = get_proj_lock(state, proj_id).await;
        let _guard = lock.lock().await;
        ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
        let settings = build_settings(state, proj_id).await;
        let settings_content = serde_json::to_vec_pretty(&settings).map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize settings failed: {e}"),
            )
        })?;
        fs::write(work_dir.join(".claw/settings.json"), settings_content)
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write settings failed: {e}"),
                )
            })?;
        let _ = fs::remove_file(work_dir.join(".claw/mcp_discovery_cache.json")).await;
        settings
    };
    let (report, loaded_names, configured_servers, status) =
        probe_mcp_load(&state.cfg.claw_bin, &work_dir, probe_timeout_seconds).await?;
    let names = mcp_server_names_from_settings(&settings);
    Ok((report, loaded_names, configured_servers, status, names))
}

pub(crate) async fn build_settings(state: &AppState, proj_id: i64) -> Value {
    if let Ok(Some(row)) = state.session_db.get_project_config(proj_id).await {
        let mut settings = project_config_apply::build_settings_json_from_row(&row);
        if let Ok(role) = state.session_db.get_project_role(proj_id).await {
            if role == master_observer::PROJECT_ROLE_MASTER {
                if let Some(token) = master_observer::master_mcp_shared_token() {
                    master_observer::merge_master_mcp_into_settings(
                        &mut settings,
                        proj_id,
                        &state.gateway_identity.gateway_base,
                        &token,
                    );
                }
            }
        }
        settings
    } else {
        json!({
            "mcpServers": serde_json::Map::new(),
            "auto_hidden_system_prompt": 1
        })
    }
}

pub(crate) async fn ensure_workspace_initialized(_claw_bin: &str, work_dir: &Path) -> Result<(), ApiError> {
    let marker = work_dir.join(".claw/.gateway_init_done");
    if fs::metadata(&marker).await.is_ok() {
        return Ok(());
    }
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("workspace init failed: {e}"),
            )
        })?;
    fs::write(marker, now_ms().to_string()).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write init marker failed: {e}"),
        )
    })?;
    Ok(())
}

pub(crate) async fn probe_mcp_load(
    claw_bin: &str,
    work_dir: &Path,
    timeout_seconds: u64,
) -> Result<(Value, Vec<String>, i64, String), ApiError> {
    let mut cmd = Command::new(claw_bin);
    cmd.current_dir(work_dir)
        .arg("mcp")
        .arg("--output-format")
        .arg("json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = timeout(Duration::from_secs(timeout_seconds), cmd.output())
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                format!("claw mcp probe timeout: {timeout_seconds}s"),
            )
        })?
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("spawn claw mcp failed: {e}"),
            )
        })?;
    let raw = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    let parsed = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({"raw": raw}));
    let loaded_names = parsed
        .get("servers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("name")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let configured_servers = parsed
        .get("configured_servers")
        .and_then(Value::as_i64)
        .unwrap_or(loaded_names.len() as i64);
    let status = parsed
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if output.status.success() {
            "ok"
        } else {
            "error"
        })
        .to_string();
    Ok((parsed, loaded_names, configured_servers, status))
}

