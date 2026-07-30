// Fragment of routes::app (include!). Author: kejiqing

#[utoipa::path(
    get,
    path = "/v1/gateway/global-settings",
    tag = "Gateway Settings",
    operation_id = "get_gateway_global_settings_handler",
    summary = "Load gateway global settings",
    responses(
        (status = 200, description = "Global settings snapshot", body = gateway_global_settings::GatewayGlobalSettingsResponse)
    )
)]
pub(crate) async fn get_gateway_global_settings_handler(
    State(state): State<AppState>,
) -> Result<Json<gateway_global_settings::GatewayGlobalSettingsResponse>, ApiError> {
    let mut body = gateway_global_settings::load_response(&state.session_db)
        .await
        .map_err(|e| session_db_err(&e))?;
    if crate::pool::interactive_backend::e2b_observe_is_enabled() {
        if let Some(tap) = body.claw_tap.as_mut() {
            let e2b_traffic = tap
                .live_base_url
                .as_deref()
                .is_some_and(gateway_e2b_observe_proxy::should_use_e2b_traffic_browser_proxy);
            if !e2b_traffic {
                *tap = gateway_claw_tap_settings::strip_compose_live_urls_for_fc_admin(tap.clone());
            }
            if let Some(client) = state.pool_clients.e2b_sandbox_client() {
                gateway_claw_tap_settings::enrich_claw_tap_observe_runtime(tap, client).await;
            }
        }
    }
    body.e2b_nas = Some(gateway_e2b_nas_settings::e2b_nas_settings_public(
        &state.cfg.work_root,
    ));
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/observe-tap/reset",
    tag = "Gateway Settings",
    operation_id = "reset_gateway_observe_tap_handler",
    summary = "Reset e2b observe tap singleton",
    responses(
        (status = 200, description = "Observe tap reset", body = gateway_e2b_observe_reset::ObserveTapResetResponse),
        (status = 503, description = "e2b sandbox client not configured"),
        (status = 502, description = "Upstream e2b observe reset failed")
    )
)]
pub(crate) async fn reset_gateway_observe_tap_handler(
    State(state): State<AppState>,
) -> Result<Json<gateway_e2b_observe_reset::ObserveTapResetResponse>, ApiError> {
    let client = state.pool_clients.e2b_sandbox_client().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "e2b sandbox client not configured",
        )
    })?;
    let mut body = gateway_e2b_observe_reset::reset_observe_tap(&state.session_db, client)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    gateway_claw_tap_settings::enrich_claw_tap_observe_runtime(&mut body.tap, client).await;
    Ok(Json(body))
}

#[utoipa::path(
    get,
    path = "/v1/gateway/global-settings/e2b-singletons",
    tag = "Gateway Settings",
    operation_id = "get_gateway_e2b_singletons_handler",
    summary = "List e2b core singleton status",
    responses(
        (status = 200, description = "Singleton status", body = gateway_e2b_singleton_api::E2bSingletonsStatusResponse)
    )
)]
pub(crate) async fn get_gateway_e2b_singletons_handler(
    State(state): State<AppState>,
) -> Result<Json<gateway_e2b_singleton_api::E2bSingletonsStatusResponse>, ApiError> {
    let body = gateway_e2b_singleton_api::load_e2b_singletons_status(
        &state.session_db,
        state.pool_clients.e2b_sandbox_client().map(|v| &**v),
    )
    .await
    .map_err(|e| session_db_err(&e))?;
    Ok(Json(body))
}

#[utoipa::path(
    get,
    path = "/v1/gateway/global-settings/e2b-templates",
    tag = "Gateway Settings",
    operation_id = "get_gateway_e2b_templates_handler",
    summary = "List e2b sandbox templates",
    responses(
        (status = 200, description = "Template catalog", body = gateway_e2b_singleton_api::E2bTemplatesListResponse),
        (status = 503, description = "e2b sandbox client not configured"),
        (status = 502, description = "Upstream e2b template list failed")
    )
)]
pub(crate) async fn get_gateway_e2b_templates_handler(
    State(state): State<AppState>,
) -> Result<Json<gateway_e2b_singleton_api::E2bTemplatesListResponse>, ApiError> {
    let client = state.pool_clients.e2b_sandbox_client().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "e2b sandbox client not configured",
        )
    })?;
    let body = gateway_e2b_singleton_api::list_e2b_templates(client)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    put,
    path = "/v1/gateway/global-settings/e2b-singleton-templates",
    tag = "Gateway Settings",
    operation_id = "put_gateway_e2b_singleton_templates_handler",
    summary = "Update e2b singleton template ids",
    request_body = gateway_e2b_singleton_api::PutE2bSingletonTemplatesInput,
    responses(
        (status = 200, description = "Updated singleton templates", body = gateway_e2b_singleton_api::PutE2bSingletonTemplatesResponse),
        (status = 400, description = "Invalid input or no templateId fields provided")
    )
)]
pub(crate) async fn put_gateway_e2b_singleton_templates_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_e2b_singleton_api::PutE2bSingletonTemplatesInput>,
) -> Result<Json<gateway_e2b_singleton_api::PutE2bSingletonTemplatesResponse>, ApiError> {
    let body = gateway_e2b_singleton_api::put_e2b_singleton_templates(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/e2b-singletons/{component}/ensure",
    tag = "Gateway Settings",
    operation_id = "ensure_gateway_e2b_singleton_handler",
    summary = "Ensure an e2b core singleton is running",
    params(
        ("component" = crate::gateway_e2b_singleton_lifecycle::E2bSingletonComponent, Path, description = "Singleton component")
    ),
    responses(
        (status = 200, description = "Singleton ensured", body = gateway_e2b_singleton_api::E2bSingletonActionResponse),
        (status = 400, description = "Invalid component"),
        (status = 503, description = "e2b sandbox client not configured"),
        (status = 502, description = "Upstream e2b ensure failed")
    )
)]
pub(crate) async fn ensure_gateway_e2b_singleton_handler(
    State(state): State<AppState>,
    AxumPath(component): AxumPath<crate::gateway_e2b_singleton_lifecycle::E2bSingletonComponent>,
) -> Result<Json<gateway_e2b_singleton_api::E2bSingletonActionResponse>, ApiError> {
    let client = state.pool_clients.e2b_sandbox_client().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "e2b sandbox client not configured",
        )
    })?;
    let body =
        gateway_e2b_singleton_api::ensure_e2b_singleton_via_api(&state.session_db, client, component)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/e2b-singletons/{component}/reset",
    tag = "Gateway Settings",
    operation_id = "reset_gateway_e2b_singleton_handler",
    summary = "Reset an e2b core singleton sandbox",
    params(
        ("component" = crate::gateway_e2b_singleton_lifecycle::E2bSingletonComponent, Path, description = "Singleton component")
    ),
    responses(
        (status = 200, description = "Singleton reset", body = gateway_e2b_singleton_api::E2bSingletonActionResponse),
        (status = 400, description = "Invalid component"),
        (status = 503, description = "e2b sandbox client not configured"),
        (status = 502, description = "Upstream e2b reset failed")
    )
)]
pub(crate) async fn reset_gateway_e2b_singleton_handler(
    State(state): State<AppState>,
    AxumPath(component): AxumPath<crate::gateway_e2b_singleton_lifecycle::E2bSingletonComponent>,
) -> Result<Json<gateway_e2b_singleton_api::E2bSingletonActionResponse>, ApiError> {
    let client = state.pool_clients.e2b_sandbox_client().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "e2b sandbox client not configured",
        )
    })?;
    let body =
        gateway_e2b_singleton_api::reset_e2b_singleton_via_api(&state.session_db, client, component)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    put,
    path = "/v1/gateway/global-settings/e2b-worker",
    tag = "Gateway Settings",
    operation_id = "put_gateway_e2b_worker_settings_handler",
    summary = "Update strict e2b worker settings",
    request_body = gateway_e2b_worker_settings::PutE2bWorkerSettingsInput,
    responses(
        (status = 200, description = "Updated worker settings", body = gateway_e2b_worker_settings::E2bWorkerSettingsPublic),
        (status = 400, description = "Invalid templateId or poolSize")
    )
)]
pub(crate) async fn put_gateway_e2b_worker_settings_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_e2b_worker_settings::PutE2bWorkerSettingsInput>,
) -> Result<Json<gateway_e2b_worker_settings::E2bWorkerSettingsPublic>, ApiError> {
    let body = gateway_e2b_worker_settings::put_e2b_worker_settings(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    let pool = state.pool_clients.clone();
    tokio::spawn(async move {
        if let Err(e) = pool.reconcile_all_project_workers().await {
            tracing::warn!(
                target: "claw_e2b_proj_worker",
                error = %e,
                "post poolSize reconcile failed (best-effort)"
            );
        }
    });
    Ok(Json(body))
}

#[utoipa::path(
    put,
    path = "/v1/gateway/global-settings/claw-tap",
    tag = "Gateway Settings",
    operation_id = "put_gateway_claw_tap_handler",
    summary = "Update clawTap global settings",
    request_body = gateway_claw_tap_settings::PutClawTapSettingsInput,
    responses(
        (status = 200, description = "Updated clawTap settings", body = gateway_claw_tap_settings::PutClawTapSettingsResponse),
        (status = 400, description = "Invalid clawTap input")
    )
)]
pub(crate) async fn put_gateway_claw_tap_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_claw_tap_settings::PutClawTapSettingsInput>,
) -> Result<Json<gateway_claw_tap_settings::PutClawTapSettingsResponse>, ApiError> {
    let body = gateway_claw_tap_settings::put_claw_tap_settings(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    let _ =
        gateway_llm_config_sync::sync_llm_runtime_from_db(&state.session_db, &state.llm_runtime)
            .await;
    if let Ok(Some(cluster)) = claw_tap_cluster_state::refresh_claw_tap_cluster_state(
        &state.session_db,
        &state.llm_runtime,
    )
    .await
    {
        *state.claw_tap_cluster.write().await = Some(cluster);
    }
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/claw-tap/probe",
    tag = "Gateway Settings",
    operation_id = "probe_gateway_claw_tap_handler",
    summary = "Probe clawTap cluster connectivity",
    request_body = gateway_claw_tap_settings::ProbeClawTapInput,
    responses(
        (status = 200, description = "Probe result", body = gateway_claw_tap_settings::ProbeClawTapResponse)
    )
)]
pub(crate) async fn probe_gateway_claw_tap_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_claw_tap_settings::ProbeClawTapInput>,
) -> Result<Json<gateway_claw_tap_settings::ProbeClawTapResponse>, ApiError> {
    let resp = gateway_claw_tap_settings::probe_claw_tap_endpoint(&state.session_db, req).await;
    Ok(Json(resp))
}

#[utoipa::path(
    put,
    path = "/v1/gateway/global-settings/strict-landlock-default",
    tag = "Gateway Settings",
    operation_id = "put_gateway_strict_landlock_default_handler",
    summary = "Update system default Landlock DSL",
    request_body = gateway_strict_landlock_settings::PutStrictLandlockDefaultInput,
    responses(
        (status = 200, description = "Updated Landlock default", body = gateway_strict_landlock_settings::PutStrictLandlockDefaultResponse),
        (status = 400, description = "Invalid Landlock DSL")
    )
)]
pub(crate) async fn put_gateway_strict_landlock_default_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_strict_landlock_settings::PutStrictLandlockDefaultInput>,
) -> Result<Json<gateway_strict_landlock_settings::PutStrictLandlockDefaultResponse>, ApiError> {
    let body =
        gateway_strict_landlock_settings::put_strict_landlock_default(&state.session_db, req)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/git-pats",
    tag = "Gateway Settings",
    operation_id = "upsert_gateway_git_pat_handler",
    summary = "Create or update a Git PAT",
    request_body = gateway_global_settings::PutGitPatInput,
    responses(
        (status = 200, description = "Git PAT saved", body = gateway_global_settings::GitPatPublic),
        (status = 400, description = "Invalid PAT input")
    )
)]
pub(crate) async fn upsert_gateway_git_pat_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_global_settings::PutGitPatInput>,
) -> Result<Json<gateway_global_settings::GitPatPublic>, ApiError> {
    let pat = gateway_global_settings::upsert_git_pat(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(pat))
}

#[utoipa::path(
    delete,
    path = "/v1/gateway/global-settings/git-pats/{pat_id}",
    tag = "Gateway Settings",
    operation_id = "delete_gateway_git_pat_handler",
    summary = "Delete a Git PAT",
    params(
        ("pat_id" = String, Path, description = "Git PAT id")
    ),
    responses(
        (status = 204, description = "Git PAT deleted"),
        (status = 404, description = "Git PAT not found"),
        (status = 400, description = "Invalid PAT id")
    )
)]
pub(crate) async fn delete_gateway_git_pat_handler(
    State(state): State<AppState>,
    AxumPath(pat_id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = gateway_global_settings::delete_git_pat(&state.session_db, &pat_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "git PAT not found"))
    }
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/admin-mcp-tokens",
    tag = "Gateway Settings",
    operation_id = "issue_gateway_admin_mcp_token_handler",
    summary = "Issue an admin MCP bearer token",
    request_body = gateway_admin_mcp_token::IssueAdminMcpTokenInput,
    responses(
        (status = 200, description = "Token issued", body = gateway_admin_mcp_token::IssueAdminMcpTokenResponse),
        (status = 400, description = "Invalid token input")
    )
)]
pub(crate) async fn issue_gateway_admin_mcp_token_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_admin_mcp_token::IssueAdminMcpTokenInput>,
) -> Result<Json<gateway_admin_mcp_token::IssueAdminMcpTokenResponse>, ApiError> {
    let body = gateway_admin_mcp_token::issue_admin_mcp_token(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    delete,
    path = "/v1/gateway/global-settings/admin-mcp-tokens/{token_id}",
    tag = "Gateway Settings",
    operation_id = "revoke_gateway_admin_mcp_token_handler",
    summary = "Revoke an admin MCP bearer token",
    params(
        ("token_id" = String, Path, description = "Admin MCP token id")
    ),
    responses(
        (status = 204, description = "Token revoked"),
        (status = 404, description = "Admin MCP token not found"),
        (status = 400, description = "Invalid token id")
    )
)]
pub(crate) async fn revoke_gateway_admin_mcp_token_handler(
    State(state): State<AppState>,
    AxumPath(token_id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let revoked = gateway_admin_mcp_token::revoke_admin_mcp_token(&state.session_db, &token_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if revoked {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "admin MCP token not found",
        ))
    }
}

#[utoipa::path(
    put,
    path = "/v1/gateway/global-settings/active-llm-config",
    tag = "Gateway Settings",
    operation_id = "put_gateway_active_llm_config_handler",
    summary = "Update active LLM config (legacy compat)",
    request_body = gateway_global_settings::PutActiveLlmConfigInput,
    responses(
        (status = 200, description = "Active LLM config saved", body = gateway_global_settings::ActiveLlmConfigPublic),
        (status = 400, description = "Invalid LLM config input"),
        (status = 500, description = "LLM runtime sync failed")
    )
)]
pub(crate) async fn put_gateway_active_llm_config_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_global_settings::PutActiveLlmConfigInput>,
) -> Result<Json<gateway_global_settings::ActiveLlmConfigPublic>, ApiError> {
    let cfg = gateway_global_settings::put_active_llm_config(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    gateway_llm_config_sync::sync_llm_runtime_from_db(&state.session_db, &state.llm_runtime)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(cfg))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/llm-models/test",
    tag = "Gateway Settings",
    operation_id = "test_gateway_llm_model_handler",
    summary = "Probe LLM upstream connectivity",
    request_body = llm_probe::LlmTestRequest,
    responses(
        (status = 200, description = "Probe result", body = llm_probe::LlmTestResponse),
        (status = 400, description = "Invalid probe input")
    )
)]
pub(crate) async fn test_gateway_llm_model_handler(
    State(state): State<AppState>,
    Json(req): Json<llm_probe::LlmTestRequest>,
) -> Result<Json<llm_probe::LlmTestResponse>, ApiError> {
    let resp = llm_probe::probe_llm_model(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/llm-models",
    tag = "Gateway Settings",
    operation_id = "upsert_gateway_llm_model_handler",
    summary = "Create or update an LLM model",
    request_body = gateway_global_settings::PutLlmModelInput,
    responses(
        (status = 200, description = "LLM model saved", body = gateway_global_settings::LlmModelPublic),
        (status = 400, description = "Invalid LLM model input"),
        (status = 500, description = "LLM runtime sync failed")
    )
)]
pub(crate) async fn upsert_gateway_llm_model_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_global_settings::PutLlmModelInput>,
) -> Result<Json<gateway_global_settings::LlmModelPublic>, ApiError> {
    let cfg = gateway_global_settings::upsert_llm_model(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    gateway_llm_config_sync::sync_llm_runtime_from_db(&state.session_db, &state.llm_runtime)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(cfg))
}

#[utoipa::path(
    delete,
    path = "/v1/gateway/global-settings/llm-models/{model_id}",
    tag = "Gateway Settings",
    operation_id = "delete_gateway_llm_model_handler",
    summary = "Delete an LLM model",
    params(
        ("model_id" = String, Path, description = "LLM model id")
    ),
    responses(
        (status = 204, description = "LLM model deleted"),
        (status = 404, description = "LLM model not found"),
        (status = 400, description = "Invalid model id")
    )
)]
pub(crate) async fn delete_gateway_llm_model_handler(
    State(state): State<AppState>,
    AxumPath(model_id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = gateway_global_settings::delete_llm_model(&state.session_db, &model_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "llm model not found"))
    }
}

#[utoipa::path(
    get,
    path = "/v1/gateway/global-settings/llm-models/{model_id}/versions",
    tag = "Gateway Settings",
    operation_id = "list_gateway_llm_model_versions_handler",
    summary = "List LLM model revision history",
    params(
        ("model_id" = String, Path, description = "LLM model id")
    ),
    responses(
        (status = 200, description = "Model versions", body = gateway_global_settings::LlmModelVersionsResponse),
        (status = 400, description = "Invalid model id or model not found")
    )
)]
pub(crate) async fn list_gateway_llm_model_versions_handler(
    State(state): State<AppState>,
    AxumPath(model_id): AxumPath<String>,
) -> Result<Json<gateway_global_settings::LlmModelVersionsResponse>, ApiError> {
    let body = gateway_global_settings::list_llm_model_versions(&state.session_db, &model_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/llm-models/{model_id}/apply",
    tag = "Gateway Settings",
    operation_id = "apply_gateway_llm_model_head_handler",
    summary = "Apply the head revision of an LLM model",
    params(
        ("model_id" = String, Path, description = "LLM model id")
    ),
    responses(
        (status = 200, description = "Model applied", body = gateway_global_settings::ApplyLlmModelResponse),
        (status = 400, description = "Invalid model id or missing apiKey"),
        (status = 500, description = "LLM runtime sync failed")
    )
)]
pub(crate) async fn apply_gateway_llm_model_head_handler(
    State(state): State<AppState>,
    AxumPath(model_id): AxumPath<String>,
) -> Result<Json<gateway_global_settings::ApplyLlmModelResponse>, ApiError> {
    let resp = apply_gateway_llm_model_with_sync(&state, &model_id, None).await?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/global-settings/llm-models/{model_id}/versions/{model_rev}/apply",
    tag = "Gateway Settings",
    operation_id = "apply_gateway_llm_model_revision_handler",
    summary = "Apply a specific LLM model revision",
    params(
        ("model_id" = String, Path, description = "LLM model id"),
        ("model_rev" = String, Path, description = "LLM model revision id")
    ),
    responses(
        (status = 200, description = "Model revision applied", body = gateway_global_settings::ApplyLlmModelResponse),
        (status = 400, description = "Invalid model id/revision or missing apiKey"),
        (status = 500, description = "LLM runtime sync failed")
    )
)]
pub(crate) async fn apply_gateway_llm_model_revision_handler(
    State(state): State<AppState>,
    AxumPath((model_id, model_rev)): AxumPath<(String, String)>,
) -> Result<Json<gateway_global_settings::ApplyLlmModelResponse>, ApiError> {
    let resp = apply_gateway_llm_model_with_sync(&state, &model_id, Some(&model_rev)).await?;
    Ok(Json(resp))
}

pub(crate) async fn apply_gateway_llm_model_with_sync(
    state: &AppState,
    model_id: &str,
    model_rev: Option<&str>,
) -> Result<gateway_global_settings::ApplyLlmModelResponse, ApiError> {
    let mut resp =
        gateway_global_settings::apply_llm_model_by_id(&state.session_db, model_id, model_rev)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    let sync =
        gateway_llm_config_sync::sync_llm_runtime_from_db(&state.session_db, &state.llm_runtime)
            .await
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if let Some(outcome) = sync.env_apply {
        resp.outcome = outcome;
    } else if let Some(path) = sync.upstream_config_file {
        resp.outcome.env_file = path;
    }
    if let Some(restart) = sync.tap_restart {
        resp.outcome.tap_restarted = restart.restarted;
        if restart.restarted {
            resp.outcome.message = restart
                .message
                .or_else(|| Some("local clawTap restarted after LLM apply".into()));
        } else if let Some(msg) = restart.message {
            resp.outcome.message = Some(msg);
        }
    }
    Ok(resp)
}
