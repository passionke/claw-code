// Fragment of routes::app (include!). Author: kejiqing

#[utoipa::path(
    get,
    path = "/v1/gateway/bootstrap/status",
    tag = "Gateway Bootstrap",
    operation_id = "get_gateway_bootstrap_status_handler",
    summary = "Cluster first-run bootstrap status",
    responses(
        (status = 200, description = "Bootstrap phases and template commands", body = gateway_cluster_bootstrap::ClusterBootstrapSnapshot),
    )
)]
pub(crate) async fn get_gateway_bootstrap_status_handler(
    State(state): State<AppState>,
) -> Result<Json<gateway_cluster_bootstrap::ClusterBootstrapSnapshot>, ApiError> {
    let client = state.pool_clients.e2b_sandbox_client().map(|v| &**v);
    let snap = gateway_cluster_bootstrap::cluster_bootstrap_status(
        &state.session_db,
        client,
        Some(&state.claw_tap_cluster),
    )
    .await
    .map_err(|e| session_db_err(&e))?;
    Ok(Json(snap))
}

#[utoipa::path(
    get,
    path = "/v1/gateway/bootstrap/env-snapshot",
    tag = "Gateway Bootstrap",
    operation_id = "get_gateway_bootstrap_env_snapshot_handler",
    summary = "Deploy env snapshot for bootstrap wizard",
    responses(
        (status = 200, description = "Env snapshot", body = gateway_bootstrap_deploy::BootstrapEnvSnapshot),
    )
)]
pub(crate) async fn get_gateway_bootstrap_env_snapshot_handler(
    State(state): State<AppState>,
) -> Json<gateway_bootstrap_deploy::BootstrapEnvSnapshot> {
    Json(gateway_bootstrap_deploy::bootstrap_env_snapshot(
        &state.session_db,
    ))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/bootstrap/apply-deploy-env",
    tag = "Gateway Bootstrap",
    operation_id = "post_gateway_bootstrap_apply_deploy_env_handler",
    summary = "Merge whitelisted keys into deploy .env",
    responses(
        (status = 200, description = "Apply outcome", body = gateway_bootstrap_deploy::BootstrapApplyDeployEnvResponse),
        (status = 400, description = "Apply failed"),
    )
)]
pub(crate) async fn post_gateway_bootstrap_apply_deploy_env_handler(
    State(state): State<AppState>,
    Json(body): Json<gateway_bootstrap_deploy::BootstrapApplyDeployEnvInput>,
) -> Result<Json<gateway_bootstrap_deploy::BootstrapApplyDeployEnvResponse>, ApiError> {
    let resp = gateway_bootstrap_deploy::apply_deploy_env(&state.session_db, body)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    // Init: .env written + set_var; replace live e2b connection (no SSH restart for URL/key). Author: kejiqing
    if let Some(client) = state.pool_clients.e2b_sandbox_client() {
        gateway_bootstrap_deploy::runtime_replace_e2b_from_env(client.as_ref())
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    }
    // Switching e2b host invalidates PG buildId pins written against the old cluster. Author: kejiqing
    if resp.templates_invalidated {
        gateway_cluster_bootstrap::invalidate_e2b_template_pins(&state.session_db)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
        let _ = gateway_cluster_bootstrap::reopen_cluster_bootstrap_wizard(&state.session_db).await;
    }
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/bootstrap/apply-llm-from-env",
    tag = "Gateway Bootstrap",
    operation_id = "post_gateway_bootstrap_apply_llm_from_env_handler",
    summary = "Apply active LLM from deploy env (CLAW_BOOTSTRAP_LLM_* / OPENAI_*)",
    responses(
        (status = 200, description = "LLM apply outcome", body = gateway_cluster_bootstrap::BootstrapApplyLlmResponse),
        (status = 400, description = "Apply failed"),
    )
)]
pub(crate) async fn post_gateway_bootstrap_apply_llm_from_env_handler(
    State(state): State<AppState>,
) -> Result<Json<gateway_cluster_bootstrap::BootstrapApplyLlmResponse>, ApiError> {
    let resp = gateway_cluster_bootstrap::apply_llm_from_env(&state.session_db, &state.llm_runtime)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/bootstrap/ensure-core",
    tag = "Gateway Bootstrap",
    operation_id = "post_gateway_bootstrap_ensure_core_handler",
    summary = "Ensure e2b core singletons after templates + LLM are ready",
    responses(
        (status = 200, description = "Ensure outcome", body = gateway_cluster_bootstrap::BootstrapEnsureCoreResponse),
        (status = 400, description = "Prerequisites not met or ensure failed"),
    )
)]
pub(crate) async fn post_gateway_bootstrap_ensure_core_handler(
    State(state): State<AppState>,
) -> Result<Json<gateway_cluster_bootstrap::BootstrapEnsureCoreResponse>, ApiError> {
    let resp = gateway_cluster_bootstrap::ensure_bootstrap_core(
        &state.session_db,
        &state.pool_clients,
        &state.llm_runtime,
        &state.claw_tap_cluster,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/bootstrap/complete",
    tag = "Gateway Bootstrap",
    operation_id = "post_gateway_bootstrap_complete_handler",
    summary = "User finished Admin wizard (验收/可选); dismiss bootstrap gate",
    responses(
        (status = 200, description = "Wizard acknowledged", body = gateway_cluster_bootstrap::BootstrapEnsureCoreResponse),
        (status = 400, description = "Phases still incomplete"),
    )
)]
pub(crate) async fn post_gateway_bootstrap_complete_handler(
    State(state): State<AppState>,
) -> Result<Json<gateway_cluster_bootstrap::BootstrapEnsureCoreResponse>, ApiError> {
    let client = state.pool_clients.e2b_sandbox_client().map(|v| &**v);
    let resp = gateway_cluster_bootstrap::complete_cluster_bootstrap_wizard(
        &state.session_db,
        client,
        Some(&state.claw_tap_cluster),
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/bootstrap/reopen",
    tag = "Gateway Bootstrap",
    operation_id = "post_gateway_bootstrap_reopen_handler",
    summary = "Clear wizard ack so Admin shows the bootstrap guide again",
    responses(
        (status = 200, description = "Wizard reopened"),
    )
)]
pub(crate) async fn post_gateway_bootstrap_reopen_handler(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    gateway_cluster_bootstrap::reopen_cluster_bootstrap_wizard(&state.session_db)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(serde_json::json!({ "reopened": true })))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/bootstrap/reset",
    tag = "Gateway Bootstrap",
    operation_id = "post_gateway_bootstrap_reset_handler",
    summary = "Clear local bootstrap pins + wizard ack for a fresh Admin init",
    responses(
        (status = 200, description = "Bootstrap reset"),
    )
)]
pub(crate) async fn post_gateway_bootstrap_reset_handler(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    gateway_cluster_bootstrap::reset_bootstrap_for_rerun(&state.session_db)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(serde_json::json!({ "reset": true })))
}

#[utoipa::path(
    post,
    path = "/v1/gateway/bootstrap/publish-templates",
    tag = "Gateway Bootstrap",
    operation_id = "post_gateway_bootstrap_publish_templates_handler",
    summary = "Accept async publish of e2b core templates from an ACR/CI image tag (returns immediately; poll GET status / publish-templates)",
    request_body = gateway_bootstrap_publish::BootstrapPublishTemplatesInput,
    responses(
        (status = 200, description = "Publish accepted or already running", body = gateway_bootstrap_publish::BootstrapPublishTemplatesResponse),
        (status = 400, description = "Invalid tag or missing script"),
    )
)]
pub(crate) async fn post_gateway_bootstrap_publish_templates_handler(
    Json(body): Json<gateway_bootstrap_publish::BootstrapPublishTemplatesInput>,
) -> Result<Json<gateway_bootstrap_publish::BootstrapPublishTemplatesResponse>, ApiError> {
    let resp = gateway_bootstrap_publish::start_publish_templates(&body)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(resp))
}

#[utoipa::path(
    get,
    path = "/v1/gateway/bootstrap/publish-templates",
    tag = "Gateway Bootstrap",
    operation_id = "get_gateway_bootstrap_publish_templates_handler",
    summary = "Current async publish job (poll this or GET /bootstrap/status; do not use a long sync POST)",
    responses(
        (status = 200, description = "Current publish job", body = gateway_bootstrap_publish::BootstrapPublishJob),
    )
)]
pub(crate) async fn get_gateway_bootstrap_publish_templates_handler(
) -> Json<gateway_bootstrap_publish::BootstrapPublishJob> {
    Json(gateway_bootstrap_publish::current_publish_job())
}

#[utoipa::path(
    get,
    path = "/v1/gateway/bootstrap/ci-image-tags",
    tag = "Gateway Bootstrap",
    operation_id = "get_gateway_bootstrap_ci_image_tags_handler",
    summary = "List ACR/CI claw-gateway-worker tags for bootstrap dropdown",
    responses(
        (status = 200, description = "Tag list", body = gateway_bootstrap_publish::BootstrapCiImageTagsResponse),
        (status = 400, description = "Registry auth or list failed"),
    )
)]
pub(crate) async fn get_gateway_bootstrap_ci_image_tags_handler(
) -> Result<Json<gateway_bootstrap_publish::BootstrapCiImageTagsResponse>, ApiError> {
    let resp = gateway_bootstrap_publish::list_ci_image_tags()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(resp))
}
