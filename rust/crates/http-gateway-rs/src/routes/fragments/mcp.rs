// Fragment of routes::app (include!). Author: kejiqing

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct InjectMcpRequest {
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    proj_id: i64,
    #[serde(rename = "mcpServers")]
    #[schema(value_type = Object)]
    mcp_servers: HashMap<String, Value>,
    replace: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct TestMcpRequest {
    #[serde(
        rename = "projId",
        alias = "proj_id",
        alias = "dsId",
        alias = "ds_id",
        default
    )]
    proj_id: Option<i64>,
    #[serde(rename = "serverName")]
    server_name: String,
    #[schema(value_type = Object)]
    config: Value,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct McpResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    request_id: String,
    #[serde(rename = "projId")]
    proj_id: i64,
    #[serde(rename = "injectedServerNames")]
    injected_server_names: Vec<String>,
    loaded: bool,
    #[serde(rename = "missingServers")]
    missing_servers: Vec<String>,
    #[serde(rename = "configuredServers")]
    configured_servers: i64,
    status: String,
    #[serde(rename = "mcpReport")]
    #[schema(value_type = Object)]
    mcp_report: Value,
}

pub(crate) fn merge_mcp_servers_json(existing: &Value, patch: HashMap<String, Value>, replace: bool) -> Value {
    if replace {
        return Value::Object(patch.into_iter().collect());
    }
    let mut obj = existing.as_object().cloned().unwrap_or_default();
    for (k, v) in patch {
        obj.insert(k, v);
    }
    Value::Object(obj)
}

pub(crate) async fn upsert_mcp_servers_for_ds(
    state: &AppState,
    proj_id: i64,
    patch: HashMap<String, Value>,
    replace: bool,
) -> Result<(), ApiError> {
    let existing = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if existing.is_some() {
        project_config_draft::ensure_draft(&state.session_db, proj_id)
            .await
            .map_err(draft_err)?;
    }
    let mut row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .unwrap_or_else(|| default_project_config_row(proj_id));
    row.mcp_servers_json = merge_mcp_servers_json(&row.mcp_servers_json, patch, replace);
    row.draft_open = true;
    row.content_rev = project_config_draft::DRAFT_CONTENT_REV.to_string();
    row.updated_at_ms = now_ms();
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
    Ok(())
}

pub(crate) async fn clear_mcp_servers_for_ds(
    state: &AppState,
    proj_id: i64,
    server_names: Option<Vec<String>>,
) -> Result<(), ApiError> {
    if state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .is_none()
    {
        return Ok(());
    }
    project_config_draft::ensure_draft(&state.session_db, proj_id)
        .await
        .map_err(draft_err)?;
    let mut row = state
        .session_db
        .get_project_config(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists");
    let mut obj = row
        .mcp_servers_json
        .as_object()
        .cloned()
        .unwrap_or_default();
    match server_names {
        Some(names) => {
            for name in names {
                obj.remove(&name);
            }
        }
        None => obj.clear(),
    }
    row.mcp_servers_json = Value::Object(obj);
    row.draft_open = true;
    row.content_rev = project_config_draft::DRAFT_CONTENT_REV.to_string();
    row.updated_at_ms = now_ms();
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
    Ok(())
}

pub(crate) fn mcp_server_names_from_settings(settings: &Value) -> Vec<String> {
    settings
        .get("mcpServers")
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
}

#[utoipa::path(
    post,
    path = "/v1/mcp/test",
    tag = "MCP",
    operation_id = "test_mcp",
    request_body = TestMcpRequest,
    responses(
        (status = 200, description = "MCP server probe result", body = mcp_probe::McpTestResponse),
        (status = 400, description = "Invalid serverName or config")
    )
)]
pub(crate) async fn test_mcp(
    Json(req): Json<TestMcpRequest>,
) -> Result<Json<mcp_probe::McpTestResponse>, ApiError> {
    let server_name = req.server_name.trim();
    if server_name.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "serverName must be non-empty",
        ));
    }
    if !req.config.is_object() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "config must be a JSON object",
        ));
    }
    if let Some(proj_id) = req.proj_id {
        if proj_id < 1 {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "projId must be >= 1",
            ));
        }
    }
    let resp = mcp_probe::probe_mcp_server(server_name, &req.config).await;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/v1/mcp/inject",
    tag = "MCP",
    operation_id = "inject_mcp",
    request_body = InjectMcpRequest,
    responses(
        (status = 200, description = "MCP servers injected and probed", body = McpResponse),
        (status = 400, description = "Invalid projId")
    )
)]
pub(crate) async fn inject_mcp(
    State(state): State<AppState>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Json(req): Json<InjectMcpRequest>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    if req.proj_id < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }
    let replace = req.replace.unwrap_or(false);
    upsert_mcp_servers_for_ds(&state, req.proj_id, req.mcp_servers, replace).await?;
    let (report, loaded_names, configured_servers, status, names) =
        apply_settings_and_probe(&state, req.proj_id, 15).await?;
    let loaded = names.iter().all(|name| loaded_names.contains(name)) && status == "ok";
    let missing_servers = names
        .iter()
        .filter(|name| !loaded_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(McpResponse {
        session_id: request_id.clone(),
        request_id,
        proj_id: req.proj_id,
        injected_server_names: names,
        loaded,
        missing_servers,
        configured_servers,
        status,
        mcp_report: report,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/mcp/injected/{proj_id}",
    tag = "MCP",
    operation_id = "get_injected_mcp",
    params(
        ("proj_id" = i64, Path, description = "Project id"),
        ProbeQuery
    ),
    responses(
        (status = 200, description = "MCP probe status for project", body = McpResponse)
    )
)]
pub(crate) async fn get_injected_mcp(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Query(query): Query<ProbeQuery>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    let timeout_seconds = query.probe_timeout_seconds.unwrap_or(15);
    let (report, loaded_names, configured_servers, status, names) =
        apply_settings_and_probe(&state, proj_id, timeout_seconds).await?;
    let loaded = names.iter().all(|name| loaded_names.contains(name)) && status == "ok";
    let missing_servers = names
        .iter()
        .filter(|name| !loaded_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(McpResponse {
        session_id: request_id.clone(),
        request_id,
        proj_id,
        injected_server_names: names,
        loaded,
        missing_servers,
        configured_servers,
        status,
        mcp_report: report,
    }))
}

#[utoipa::path(
    delete,
    path = "/v1/mcp/injected/{proj_id}",
    tag = "MCP",
    operation_id = "delete_injected_mcp",
    params(
        ("proj_id" = i64, Path, description = "Project id"),
        DeleteQuery
    ),
    responses(
        (status = 200, description = "MCP servers cleared and re-probed", body = McpResponse)
    )
)]
pub(crate) async fn delete_injected_mcp(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    let targets = query.server_names.as_ref().map(|names| {
        names
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    });
    clear_mcp_servers_for_ds(&state, proj_id, targets).await?;
    let timeout_seconds = query.probe_timeout_seconds.unwrap_or(15);
    let (report, loaded_names, configured_servers, status, names) =
        apply_settings_and_probe(&state, proj_id, timeout_seconds).await?;
    let loaded = names.iter().all(|name| loaded_names.contains(name)) && status == "ok";
    let missing_servers = names
        .iter()
        .filter(|name| !loaded_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(McpResponse {
        session_id: request_id.clone(),
        request_id,
        proj_id,
        injected_server_names: names,
        loaded,
        missing_servers,
        configured_servers,
        status,
        mcp_report: report,
    }))
}

