// Fragment of routes::app (include!). Author: kejiqing

#[utoipa::path(
    post,
    path = "/v1/admin/mcp",
    tag = "MCP",
    operation_id = "admin_mcp_http_handler",
    summary = "Admin MCP JSON-RPC over HTTP",
    description = "Streamable MCP protocol endpoint for admin tooling. Request and response bodies follow MCP JSON-RPC framing (not a fixed REST schema). Accepts `application/json` MCP JSON-RPC requests.",
    request_body(content = inline(Object), content_type = "application/json", description = "MCP JSON-RPC request body"),
    responses(
        (status = 200, description = "MCP JSON-RPC response", content_type = "application/json")
    )
)]
pub(crate) async fn admin_mcp_http_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let backend = GatewayAdminMcpSolveBackend {
        state: state.clone(),
    };
    admin_mcp_http::handle_admin_mcp_post(
        &backend.state.session_db,
        &backend.state.cfg.work_root,
        &backend,
        state.pool_clients.nas_layout(),
        &state.pool_clients,
        &headers,
        body,
    )
    .await
}

#[derive(Clone)]
pub(crate) struct GatewayAdminMcpSolveBackend {
    state: AppState,
}

pub(crate) fn admin_mcp_input_to_solve_request(input: admin_mcp_solve::AdminMcpSolveInput) -> SolveRequest {
    SolveRequest {
        proj_id: input.proj_id,
        user_prompt: input.user_prompt,
        session_id: input.session_id,
        model: input.model,
        timeout_seconds: input.timeout_seconds,
        extra_session: input.extra_session,
        allowed_tools: input.allowed_tools,
        attachments: input.attachments,
    }
}

pub(crate) async fn admin_mcp_run_solve_sync(
    state: &AppState,
    req: SolveRequest,
) -> Result<SolveResponse, ApiError> {
    let effective = session_merge::trim_session_id(req.session_id.as_deref())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let client_origin = Some(client_origin::CLIENT_ORIGIN_ADMIN_MCP.to_string());
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
            request_id: effective,
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
    result
}

pub(crate) async fn admin_mcp_run_solve_async(
    state: AppState,
    req: SolveRequest,
) -> Result<SolveAsyncResponse, ApiError> {
    let http_request_id = HttpRequestId(Uuid::new_v4().to_string());
    let id_kind = session_merge::HttpRequestIdKind::Generated;
    let client_origin = Some(client_origin::CLIENT_ORIGIN_ADMIN_MCP.to_string());
    enqueue_solve_async(
        state,
        http_request_id,
        id_kind,
        req,
        "/v1/admin/mcp",
        client_origin,
    )
    .await
}

pub(crate) async fn admin_mcp_load_task_json(state: &AppState, task_id: &str) -> Result<Value, ApiError> {
    refresh_task_progress(state, task_id).await;
    let (mut task, proj_id) = try_load_task_record(state, task_id).await?.ok_or_else(|| {
        ApiError::new(StatusCode::NOT_FOUND, format!("task not found: {task_id}"))
    })?;
    let session_home = resolve_session_home_path(state, proj_id, &task.session_id).await;
    let progress_snap = load_turn_progress_snapshot(
        state,
        &task.turn_id,
        &task.session_id,
        proj_id,
        &task.status,
        50,
    )
    .await?;
    let (plan_title, todos) = pool_consumer_resolve::plan_fields_from_snapshot(&progress_snap);
    task.progress_history = progress_snap.events;
    task.plan_title = plan_title;
    task.todos = todos;
    let queue = {
        let tasks = state.tasks.lock().await;
        gateway_queue_snapshot(&tasks)
    };
    let trace_paths = session_home
        .as_ref()
        .map(|home| discover_trace_paths(home, &state.cfg.work_root, &task.session_id))
        .unwrap_or_default();
    let tool = trace_tail_suggests_tool_call(&trace_paths);
    let desc = resolve_current_task_desc(
        &task.status,
        &queue,
        tool,
        progress_snap.task_progress.as_ref(),
    );
    if matches!(task.status.as_str(), "queued" | "running") {
        task.current_task_desc = desc;
        task.progress_updated_at_ms = progress_snap
            .task_progress
            .as_ref()
            .map(|p| p.updated_at_ms);
    } else if task.current_task_desc.is_none() {
        task.current_task_desc = desc;
        task.progress_updated_at_ms = progress_snap
            .task_progress
            .as_ref()
            .map(|p| p.updated_at_ms);
    }
    let turn_id = task.turn_id.clone();
    let session_id = task.session_id.clone();
    apply_turn_pool_fields_from_db(&state.session_db, &turn_id, &session_id, proj_id, &mut task)
        .await;
    serde_json::to_value(&task).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize task record: {e}"),
        )
    })
}

pub(crate) fn api_error_to_mcp_string(err: ApiError) -> String {
    format!("{}: {}", err.status.as_u16(), err.message)
}

#[async_trait::async_trait]
impl admin_mcp_solve::AdminMcpSolveBackend for GatewayAdminMcpSolveBackend {
    async fn gateway_solve_sync(
        &self,
        input: admin_mcp_solve::AdminMcpSolveInput,
    ) -> Result<Value, String> {
        let req = admin_mcp_input_to_solve_request(input);
        let resp = admin_mcp_run_solve_sync(&self.state, req)
            .await
            .map_err(api_error_to_mcp_string)?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    }

    async fn gateway_solve_async(
        &self,
        input: admin_mcp_solve::AdminMcpSolveInput,
    ) -> Result<Value, String> {
        let req = admin_mcp_input_to_solve_request(input);
        let resp = admin_mcp_run_solve_async(self.state.clone(), req)
            .await
            .map_err(api_error_to_mcp_string)?;
        serde_json::to_value(resp).map_err(|e| e.to_string())
    }

    async fn gateway_task_get(&self, task_id: &str) -> Result<Value, String> {
        admin_mcp_load_task_json(&self.state, task_id)
            .await
            .map_err(api_error_to_mcp_string)
    }
}

