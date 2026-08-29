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
