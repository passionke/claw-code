// Fragment of routes::app (include!). Author: kejiqing

#[derive(Debug, Clone, Copy, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
#[schema(example = "good")]
pub(crate) enum AgentFeedbackValue {
    /// Positive feedback for the turn.
    Good,
    /// Negative feedback for the turn.
    Bad,
}

impl AgentFeedbackValue {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Bad => "bad",
        }
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct AgentFeedbackPostRequest {
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    #[schema(example = 1_i64)]
    proj_id: i64,
    #[serde(rename = "sessionId")]
    #[schema(example = "sess_demo")]
    session_id: String,
    #[serde(rename = "turnId")]
    #[schema(example = "T_0123456789abcdef0123456789abcdef")]
    turn_id: String,
    /// Enum: `good` | `bad`.
    feedback: AgentFeedbackValue,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AgentFeedbackGetQuery {
    #[serde(rename = "sessionId")]
    #[param(rename = "sessionId")]
    session_id: String,
    #[serde(flatten)]
    project_id_query: project_id::ProjectIdQuery,
}

impl AgentFeedbackGetQuery {
    fn resolved_proj_id(&self) -> Option<i64> {
        project_id::parse_project_id_query(&self.project_id_query)
    }
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct AgentFeedbackPostResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "turnId")]
    turn_id: String,
    feedback: AgentFeedbackValue,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: i64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct AgentFeedbackGetResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "projId")]
    proj_id: i64,
    items: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BizAdviceReportBakQuery {
    task_id: String,
    /// `true` 时返回 `text/event-stream`（`biz.report.start` / `delta` / `done`），走 LLM 润色。
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BizAdviceReportQuery {
    #[serde(rename = "sessionId")]
    #[param(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "turnId")]
    #[param(rename = "turnId")]
    turn_id: String,
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    #[param(rename = "projId")]
    proj_id: i64,
    /// `true` 时走与 `biz_advice_report_bak` 相同的 LLM 润色 SSE；默认 `false` 返回 JSON。
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct DevBizReportSeedRequest {
    #[serde(rename = "taskId")]
    task_id: Option<String>,
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    proj_id: i64,
    #[serde(rename = "outputText", default)]
    output_text: String,
    #[serde(rename = "outputJson")]
    #[schema(value_type = Object)]
    output_json: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct BizAdviceReportResponse {
    #[serde(rename = "taskId")]
    task_id: String,
    #[serde(rename = "sourceRequestId")]
    source_request_id: String,
    #[serde(rename = "sourceDsId")]
    source_proj_id: i64,
    #[serde(rename = "sourceStatus")]
    source_status: String,
    #[serde(rename = "reportText")]
    report_text: String,
    #[serde(rename = "reportJson")]
    #[schema(value_type = Object)]
    report_json: Option<Value>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct DevBizReportSeedResponse {
    #[serde(rename = "taskId")]
    task_id: String,
    #[serde(rename = "bizAdviceReportStreamUrl")]
    biz_advice_report_stream_url: String,
}

#[derive(Debug, Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DeleteQuery {
    server_names: Option<String>,
    #[serde(rename = "probe_timeout_seconds")]
    #[param(rename = "probe_timeout_seconds")]
    probe_timeout_seconds: Option<u64>,
}

#[utoipa::path(
    post,
    path = "/v1/dev/biz_report_seed_task",
    tag = "Sessions",
    operation_id = "dev_seed_biz_report_task",
    summary = "Dev-only: seed succeeded task for biz report testing",
    request_body = DevBizReportSeedRequest,
    responses(
        (status = 200, description = "Seeded task id and stream URL", body = DevBizReportSeedResponse),
        (status = 404, description = "Disabled unless CLAW_GATEWAY_DEV_BIZ_REPORT_SEED=1")
    )
)]
pub(crate) async fn dev_seed_biz_report_task(
    State(state): State<AppState>,
    Json(body): Json<DevBizReportSeedRequest>,
) -> Result<Json<DevBizReportSeedResponse>, ApiError> {
    if std::env::var("CLAW_GATEWAY_DEV_BIZ_REPORT_SEED")
        .ok()
        .as_deref()
        != Some("1")
    {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not found"));
    }
    if body.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let tid = body
        .task_id
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map_or_else(|| Uuid::new_v4().simple().to_string(), ToString::to_string);
    let work_dir = proj_work_dir(&state.cfg.work_root, body.proj_id);
    let now = now_ms();
    let output_text = if body.output_text.trim().is_empty() {
        "mock raw boss output for polish".to_string()
    } else {
        body.output_text.clone()
    };
    let seed_turn_id = turn_id::mint_turn_id();
    let result = SolveResponse {
        session_id: tid.clone(),
        request_id: tid.clone(),
        session_home_rel: format!("proj_{}/sessions/dev-seed", body.proj_id),
        proj_id: body.proj_id,
        work_dir: work_dir.to_string_lossy().to_string(),
        duration_ms: 0,
        claw_exit_code: 0,
        output_text,
        output_json: body.output_json.clone(),
        turn_id: seed_turn_id.clone(),
    };
    let record = TaskRecord {
        task_id: tid.clone(),
        session_id: tid.clone(),
        request_id: tid.clone(),
        proj_id: body.proj_id,
        status: "succeeded".to_string(),
        created_at_ms: now,
        started_at_ms: Some(now),
        finished_at_ms: Some(now),
        current_task_desc: Some("分析完成".to_string()),
        progress_updated_at_ms: Some(now),
        result: Some(result),
        error: None,
        turn_id: seed_turn_id.clone(),
        progress_history: Vec::new(),
        has_report: false,
        report_time_ms: None,
        plan_title: None,
        todos: Vec::new(),
        pool_id: None,
        worker_name: None,
        worker_profile: None,
        worker_exec_user: None,
        gateway_id: None,
        gateway_base: None,
    };
    {
        let mut tasks = state.tasks.lock().await;
        tasks.insert(
            tid.clone(),
            TaskInner {
                record,
                cancel: None,
                proj_id: body.proj_id,
            },
        );
    }
    let stream_url = format!(
        "/v1/biz_advice_report?sessionId={tid}&turnId={seed_turn_id}&projId={}&stream=true",
        body.proj_id
    );
    Ok(Json(DevBizReportSeedResponse {
        task_id: tid,
        biz_advice_report_stream_url: stream_url,
    }))
}

pub(crate) struct BizReportDbCtx {
    task_id: String,
    turn_id: String,
    status: String,
}

pub(crate) async fn resolve_biz_report_from_db(
    state: &AppState,
    query: &BizAdviceReportQuery,
) -> Result<BizReportDbCtx, ApiError> {
    let belongs = state
        .session_db
        .turn_belongs_to_session(&query.turn_id, &query.session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if !belongs {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!(
                "no turn for sessionId={} turnId={} projId={}",
                query.session_id, query.turn_id, query.proj_id
            ),
        ));
    }
    let status = state
        .session_db
        .get_turn_status(&query.turn_id, &query.session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!(
                    "no turn row for sessionId={} turnId={}",
                    query.session_id, query.turn_id
                ),
            )
        })?;
    Ok(BizReportDbCtx {
        task_id: query.session_id.clone(),
        turn_id: query.turn_id.clone(),
        status,
    })
}

pub(crate) async fn load_turn_report_body_from_db(
    state: &AppState,
    query: &BizAdviceReportQuery,
) -> Result<String, ApiError> {
    let report_message = state
        .session_db
        .get_turn_report_message(&query.turn_id, &query.session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let output_json = state
        .session_db
        .get_turn_output_json(&query.turn_id, &query.session_id, query.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    report_body_from_persisted(report_message.as_deref(), output_json.as_ref()).ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            format!(
                "turn {} has no persisted report message (outputJson.message)",
                query.turn_id
            ),
        )
    })
}

pub(crate) fn biz_report_json_response(
    ctx: &BizReportDbCtx,
    query: &BizAdviceReportQuery,
    body: &str,
) -> Response {
    Json(BizAdviceReportResponse {
        task_id: ctx.task_id.clone(),
        source_request_id: ctx.task_id.clone(),
        source_proj_id: query.proj_id,
        source_status: ctx.status.clone(),
        report_text: body.to_string(),
        report_json: Some(json!({ "message": body })),
    })
    .into_response()
}

pub(crate) async fn biz_advice_report_legacy_polish_mode(
    state: &AppState,
    query: &BizAdviceReportQuery,
    ctx: &BizReportDbCtx,
) -> Result<Response, ApiError> {
    if ctx.status != "succeeded" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "turn not finished yet (status: {}); legacy spill mode serves report after succeeded only",
                ctx.status
            ),
        ));
    }
    let report_body = load_turn_report_body_from_db(state, query).await?;
    let skill_work_dir = proj_work_dir(&state.cfg.work_root, BOSS_REPORT_SKILL_PROJ_ID);
    ensure_workspace_initialized(&state.cfg.claw_bin, &skill_work_dir).await?;
    let instructions = load_boss_report_writer_instructions(&skill_work_dir).await;
    let prompt = build_biz_advice_polish_prompt(&instructions, &report_body);
    let request_id = Uuid::new_v4().simple().to_string();
    let timeout_seconds = state.cfg.default_timeout_seconds;
    tracing::info!(
        target: "claw_live_report",
        component = "biz_advice_report",
        phase = "route",
        route = "legacy_spill_polish_llm",
        turn_id = %ctx.turn_id,
        session_id = %query.session_id,
        proj_id = query.proj_id,
        stream = query.stream,
        "biz_advice_report — legacy spill mode LLM polish"
    );
    if query.stream {
        let meta = BizAdviceReportPayload {
            task_id: ctx.task_id.clone(),
            source_request_id: ctx.task_id.clone(),
            source_proj_id: query.proj_id,
            source_status: ctx.status.clone(),
            report_text: None,
            report_json: None,
        };
        let task_id = meta.task_id.clone();
        let polish_ds = state.cfg.report_polish_deepseek.clone();
        return Ok(biz_report_llm_stream_response(
            &task_id,
            meta,
            prompt,
            request_id,
            timeout_seconds,
            polish_ds,
        ));
    }
    let polish_ds = state.cfg.report_polish_deepseek.clone();
    let (report_text, report_json) = tokio::task::spawn_blocking(move || {
        run_gateway_biz_polish_llm(
            &prompt,
            None,
            timeout_seconds,
            &request_id,
            None::<fn(&str)>,
            polish_ds.as_ref(),
        )
    })
    .await
    .map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("polish task join failed: {e}"),
        )
    })?
    .map_err(map_gateway_solve_turn_err)?;
    let (report_text, report_json) = sanitize_biz_report_parts(&report_text, report_json);
    Ok(Json(BizAdviceReportResponse {
        task_id: ctx.task_id.clone(),
        source_request_id: ctx.task_id.clone(),
        source_proj_id: query.proj_id,
        source_status: ctx.status.clone(),
        report_text,
        report_json,
    })
    .into_response())
}

#[utoipa::path(
    get,
    path = "/v1/biz_advice_report",
    tag = "Sessions",
    operation_id = "get_biz_advice_report",
    params(BizAdviceReportQuery),
    responses(
        (status = 200, description = "Report JSON when stream=false", body = BizAdviceReportResponse),
        (status = 200, description = "Live report SSE when stream=true", content_type = "text/event-stream"),
        (status = 404, description = "Turn or report not found")
    )
)]
pub(crate) async fn get_biz_advice_report(
    State(state): State<AppState>,
    Query(query): Query<BizAdviceReportQuery>,
) -> Result<Response, ApiError> {
    let ctx = resolve_biz_report_from_db(&state, &query).await?;

    if state.cfg.live_biz_report_spill_enabled {
        return biz_advice_report_legacy_polish_mode(&state, &query, &ctx).await;
    }

    if matches!(ctx.status.as_str(), "succeeded" | "failed" | "cancelled") {
        if let Ok(body) = load_turn_report_body_from_db(&state, &query).await {
            if !body.trim().is_empty() {
                if query.stream {
                    tracing::info!(
                        target: "claw_live_report",
                        component = "biz_advice_report",
                        phase = "route",
                        route = "db_snapshot_sse",
                        turn_id = %ctx.turn_id,
                        session_id = %query.session_id,
                        proj_id = query.proj_id,
                        status = %ctx.status,
                        "biz_advice_report stream — terminal snapshot from gateway_turns (no pool HTTP)"
                    );
                    let payload = BizAdviceReportPayload {
                        task_id: ctx.task_id.clone(),
                        source_request_id: ctx.task_id.clone(),
                        source_proj_id: query.proj_id,
                        source_status: ctx.status.clone(),
                        report_text: Some(body.clone()),
                        report_json: Some(json!({ "message": body })),
                    };
                    return Ok(db_snapshot_report_sse_response(
                        &ctx.task_id,
                        payload,
                        &body,
                    ));
                }
                return Ok(biz_report_json_response(&ctx, &query, &body));
            }
        }
        if !query.stream {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!(
                    "turn {} has no persisted report (status={})",
                    query.turn_id, ctx.status
                ),
            ));
        }
    }

    if query.stream && matches!(ctx.status.as_str(), "running" | "queued") {
        match crate::gateway_owner_proxy::resolve_turn_owner_proxy_base(
            &state.session_db,
            state.gateway_identity.as_ref(),
            &ctx.turn_id,
            &query.session_id,
            query.proj_id,
        )
        .await
        {
            Ok(Some(owner_base)) => {
                tracing::info!(
                    target: "claw_live_report",
                    component = "biz_advice_report",
                    phase = "route",
                    route = "owner_gateway_proxy_sse",
                    turn_id = %ctx.turn_id,
                    owner_base = %owner_base,
                    "biz_advice_report stream — reverse proxy to owning gateway"
                );
                let path = format!(
                    "/v1/biz_advice_report?sessionId={}&turnId={}&projId={}&stream=true",
                    query.session_id, query.turn_id, query.proj_id
                );
                let headers = HeaderMap::new();
                return crate::gateway_owner_proxy::proxy_to_owner_gateway(
                    &owner_base,
                    "GET",
                    &path,
                    &headers,
                )
                .await
                .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e));
            }
            Ok(None) => {}
            Err(e) => {
                return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e));
            }
        }
        // Router and normal turns share one Hub path. Specialist body reaches the
        // router turn Hub via worker `delegate_project_tool` passthrough. Author: kejiqing
        tracing::info!(
            target: "claw_live_report",
            component = "biz_advice_report",
            phase = "route",
            route = "gateway_hub_sse",
            turn_id = %ctx.turn_id,
            session_id = %query.session_id,
            proj_id = query.proj_id,
            status = %ctx.status,
            "biz_advice_report stream — gateway LiveReportHub (sandbox exec NDJSON relay)"
        );
        return Ok(pool::live_report_sse_response(
            Arc::clone(&state.live_report_hub),
            &ctx.turn_id,
            ctx.task_id.clone(),
            ctx.task_id.clone(),
            query.proj_id,
        ));
    }

    get_biz_advice_report_bak(
        State(state),
        Query(BizAdviceReportBakQuery {
            task_id: ctx.task_id,
            stream: query.stream,
        }),
    )
    .await
}

#[utoipa::path(
    get,
    path = "/v1/biz_advice_report_bak",
    tag = "Sessions",
    operation_id = "get_biz_advice_report_bak",
    params(BizAdviceReportBakQuery),
    responses(
        (status = 200, description = "Polished report JSON when stream=false", body = BizAdviceReportResponse),
        (status = 200, description = "LLM polish SSE when stream=true", content_type = "text/event-stream"),
        (status = 400, description = "Task not succeeded")
    )
)]
pub(crate) async fn get_biz_advice_report_bak(
    State(state): State<AppState>,
    Query(query): Query<BizAdviceReportBakQuery>,
) -> Result<Response, ApiError> {
    let (task, _ds_id) = try_load_task_record(&state, &query.task_id)
        .await?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("task not found: {}", query.task_id),
            )
        })?;
    let source_status = task.status.clone();
    if source_status != "succeeded" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "task {} is not succeeded yet (status: {})",
                query.task_id, source_status
            ),
        ));
    }
    let source_result = task.result.ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "task {} has no result yet (status: {})",
                query.task_id, source_status
            ),
        )
    })?;
    if source_result.claw_exit_code != 0 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "task {} did not complete successfully (clawExitCode: {})",
                query.task_id, source_result.claw_exit_code
            ),
        ));
    }
    let report_body = report_body_from_solve_output(
        &source_result.output_text,
        source_result.output_json.as_ref(),
    )
    .map_err(|detail| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("task {} has empty report message: {detail}", query.task_id),
        )
    })?;
    let proj_id = source_result.proj_id;
    let skill_work_dir = proj_work_dir(&state.cfg.work_root, BOSS_REPORT_SKILL_PROJ_ID);
    ensure_workspace_initialized(&state.cfg.claw_bin, &skill_work_dir).await?;
    let instructions = load_boss_report_writer_instructions(&skill_work_dir).await;
    let prompt = build_biz_advice_polish_prompt(&instructions, &report_body);
    let request_id = Uuid::new_v4().simple().to_string();
    let timeout_seconds = state.cfg.default_timeout_seconds;
    if query.stream {
        let meta = BizAdviceReportPayload {
            task_id: query.task_id.clone(),
            source_request_id: task.request_id.clone(),
            source_proj_id: proj_id,
            source_status: source_status.clone(),
            report_text: None,
            report_json: None,
        };
        let task_id = meta.task_id.clone();
        let polish_ds = state.cfg.report_polish_deepseek.clone();
        return Ok(biz_report_llm_stream_response(
            &task_id,
            meta,
            prompt,
            request_id,
            timeout_seconds,
            polish_ds,
        ));
    }
    let polish_ds = state.cfg.report_polish_deepseek.clone();
    let (report_text, report_json) = tokio::task::spawn_blocking(move || {
        run_gateway_biz_polish_llm(
            &prompt,
            None,
            timeout_seconds,
            &request_id,
            None::<fn(&str)>,
            polish_ds.as_ref(),
        )
    })
    .await
    .map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("polish task join failed: {e}"),
        )
    })?
    .map_err(map_gateway_solve_turn_err)?;
    let (report_text, report_json) = sanitize_biz_report_parts(&report_text, report_json);
    Ok(Json(BizAdviceReportResponse {
        task_id: query.task_id,
        source_request_id: task.request_id,
        source_proj_id: proj_id,
        source_status,
        report_text,
        report_json,
    })
    .into_response())
}

pub(crate) fn biz_report_llm_stream_response(
    task_id: &str,
    meta_done: BizAdviceReportPayload,
    prompt: String,
    request_id: String,
    timeout_seconds: u64,
    report_polish_deepseek: Option<ReportPolishDeepseek>,
) -> Response {
    let (tx, rx) = mpsc::unbounded_channel::<BizReportStreamMsg>();
    tokio::spawn(async move {
        let mut export_sanitizer = ReportExportSanitizer::new(true);
        let mut send_delta = |delta: &str| {
            let clean = export_sanitizer.push_chunk(delta);
            if !clean.is_empty() {
                let _ = tx.send(BizReportStreamMsg::Delta(
                    crate::biz_advice_report::BizReportDeltaChunk {
                        text: clean,
                        emit_seq: None,
                    },
                ));
            }
        };
        match run_gateway_biz_polish_llm_async(
            &prompt,
            None,
            timeout_seconds,
            &request_id,
            Some(&mut send_delta),
            report_polish_deepseek.as_ref(),
        )
        .await
        {
            Ok((output_text, output_json)) => {
                let mut done = BizAdviceReportPayload {
                    task_id: meta_done.task_id,
                    source_request_id: meta_done.source_request_id,
                    source_proj_id: meta_done.source_proj_id,
                    source_status: meta_done.source_status,
                    report_text: Some(sanitize_external_report_text(&output_text)),
                    report_json: output_json,
                };
                sanitize_report_payload(&mut done);
                let _ = tx.send(BizReportStreamMsg::Done(done));
            }
            Err(e) => {
                let _ = tx.send(BizReportStreamMsg::Error(e.message));
            }
        }
    });
    let no_buffer = header::HeaderName::from_static("x-accel-buffering");
    let no_buffer_val = HeaderValue::from_static("no");
    (
        AppendHeaders([(no_buffer, no_buffer_val)]),
        Sse::new(biz_report_sse_event_stream(task_id, rx)).keep_alive(KeepAlive::default()),
    )
        .into_response()
}

#[utoipa::path(
    post,
    path = "/v1/agent/feedback",
    tag = "Sessions",
    operation_id = "post_agent_feedback",
    request_body(
        content = AgentFeedbackPostRequest,
        description = "feedback is an enum: good | bad",
        examples(
            ("Good" = (
                summary = "Mark turn as good",
                value = json!({
                    "projId": 1,
                    "sessionId": "sess_demo",
                    "turnId": "T_0123456789abcdef0123456789abcdef",
                    "feedback": "good"
                })
            )),
            ("Bad" = (
                summary = "Mark turn as bad",
                value = json!({
                    "projId": 1,
                    "sessionId": "sess_demo",
                    "turnId": "T_0123456789abcdef0123456789abcdef",
                    "feedback": "bad"
                })
            ))
        )
    ),
    responses(
        (status = 200, description = "Feedback upserted", body = AgentFeedbackPostResponse),
        (status = 400, description = "Invalid session, turn, or feedback value"),
        (status = 403, description = "Admin UI may only submit feedback for admin-origin turns")
    )
)]
pub(crate) async fn post_agent_feedback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AgentFeedbackPostRequest>,
) -> Result<Json<AgentFeedbackPostResponse>, ApiError> {
    if body.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let session_id = body.session_id.trim();
    if session_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId must be non-empty",
        ));
    }
    let turn = body.turn_id.trim();
    if !turn_id::validate_turn_id(turn) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "turnId must match T_<32 hex>",
        ));
    }
    let feedback = body.feedback;
    if !state
        .session_db
        .session_exists(session_id, body.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unknown sessionId for projId",
        ));
    }
    if !state
        .session_db
        .turn_belongs_to_session(turn, session_id, body.proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unknown turnId for session",
        ));
    }
    if client_origin_from_headers(&headers) == Some(client_origin::CLIENT_ORIGIN_GATEWAY_ADMIN) {
        let turn_origin = state
            .session_db
            .get_turn_client_origin(turn, session_id, body.proj_id)
            .await
            .map_err(|e| session_db_err(&e))?;
        if turn_origin.as_deref() != Some(client_origin::CLIENT_ORIGIN_GATEWAY_ADMIN) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "admin UI may only submit feedback for admin-origin turns",
            ));
        }
    }
    let updated_at_ms = now_ms();
    state
        .session_db
        .upsert_feedback(
            session_id,
            body.proj_id,
            turn,
            feedback.as_str(),
            updated_at_ms,
        )
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(AgentFeedbackPostResponse {
        session_id: session_id.to_string(),
        proj_id: body.proj_id,
        turn_id: turn.to_string(),
        feedback,
        updated_at_ms,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/agent/feedback",
    tag = "Sessions",
    operation_id = "get_agent_feedback",
    params(AgentFeedbackGetQuery),
    responses(
        (status = 200, description = "Feedback map keyed by turnId", body = AgentFeedbackGetResponse),
        (status = 400, description = "Missing projId or sessionId"),
        (status = 404, description = "Unknown sessionId")
    )
)]
pub(crate) async fn get_agent_feedback(
    State(state): State<AppState>,
    Query(query): Query<AgentFeedbackGetQuery>,
) -> Result<Json<AgentFeedbackGetResponse>, ApiError> {
    let Some(proj_id) = query.resolved_proj_id() else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId or proj_id query parameter is required",
        ));
    };
    if proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let session_id = query.session_id.trim();
    if session_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId must be non-empty",
        ));
    }
    if !state
        .session_db
        .session_exists(session_id, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown sessionId for projId",
        ));
    }
    let items = state
        .session_db
        .list_feedback(session_id, proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(AgentFeedbackGetResponse {
        session_id: session_id.to_string(),
        proj_id,
        items,
    }))
}

