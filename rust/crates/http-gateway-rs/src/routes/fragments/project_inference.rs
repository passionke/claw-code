// Fragment of routes::app (include!). Author: kejiqing

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/e2b-worker",
    tag = "ProjectInference",
    operation_id = "get_project_e2b_worker_handler",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "E2B worker pool status", body = gateway_project_e2b_worker::ProjectE2bWorkerStatusResponse),
        (status = 503, description = "E2B sandbox client not configured"),
        (status = 502, description = "Upstream E2B error")
    )
)]
pub(crate) async fn get_project_e2b_worker_handler(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<gateway_project_e2b_worker::ProjectE2bWorkerStatusResponse>, ApiError> {
    let client = state.pool_clients.e2b_sandbox_client().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "e2b sandbox client not configured",
        )
    })?;
    let body = gateway_project_e2b_worker::get_project_e2b_worker_status(
        state.pool_clients.e2b_worker_registry(),
        &state.session_db,
        client,
        proj_id,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/projects/{proj_id}/e2b-worker/reset",
    tag = "ProjectInference",
    operation_id = "reset_project_e2b_worker_handler",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("slotIndex" = Option<u32>, Query, description = "Optional worker slot to reset")
    ),
    responses(
        (status = 200, description = "Worker reset result", body = gateway_project_e2b_worker::ProjectE2bWorkerResetResponse),
        (status = 503, description = "E2B sandbox client not configured"),
        (status = 502, description = "Upstream E2B error")
    )
)]
pub(crate) async fn reset_project_e2b_worker_handler(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Query(query): Query<ResetProjectE2bWorkerQuery>,
) -> Result<Json<gateway_project_e2b_worker::ProjectE2bWorkerResetResponse>, ApiError> {
    let client = state.pool_clients.e2b_sandbox_client().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "e2b sandbox client not configured",
        )
    })?;
    let body = gateway_project_e2b_worker::reset_project_e2b_worker(
        state.pool_clients.e2b_worker_registry(),
        &state.session_db,
        client,
        proj_id,
        query.slot_index,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/inference",
    tag = "ProjectInference",
    operation_id = "get_project_inference_handler",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Project inference settings", body = gateway_project_llm::ProjectInferenceSettingsResponse),
        (status = 400, description = "Invalid projId")
    )
)]
pub(crate) async fn get_project_inference_handler(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<gateway_project_llm::ProjectInferenceSettingsResponse>, ApiError> {
    let mut body = gateway_project_llm::load_project_inference_settings(&state.session_db, proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if let (Some(client), Some(sid)) = (
        state.pool_clients.e2b_sandbox_client(),
        body.observe.sandbox_id.clone(),
    ) {
        body.observe.e2b_observe_sandbox_running = Some(client.sandbox_running(&sid).await);
    }
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/projects/{proj_id}/inference/llm-models",
    tag = "ProjectInference",
    operation_id = "upsert_project_llm_model_handler",
    params(("proj_id" = i64, Path, description = "Project ID")),
    request_body = gateway_global_settings::PutLlmModelInput,
    responses(
        (status = 200, description = "LLM model upserted", body = gateway_global_settings::LlmModelPublic),
        (status = 400, description = "Invalid payload")
    )
)]
pub(crate) async fn upsert_project_llm_model_handler(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Json(req): Json<gateway_global_settings::PutLlmModelInput>,
) -> Result<Json<gateway_global_settings::LlmModelPublic>, ApiError> {
    let cfg = gateway_project_llm::upsert_project_llm_model(&state.session_db, proj_id, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if let Some(client) = state.pool_clients.e2b_sandbox_client() {
        let _ = gateway_project_observe::ensure_project_observe(&state.session_db, client, proj_id)
            .await;
    }
    Ok(Json(cfg))
}

#[utoipa::path(
    post,
    path = "/v1/projects/{proj_id}/inference/llm-models/test",
    tag = "ProjectInference",
    operation_id = "test_project_llm_model_handler",
    params(("proj_id" = i64, Path, description = "Project ID")),
    request_body = llm_probe::LlmTestRequest,
    responses(
        (status = 200, description = "LLM probe result", body = llm_probe::LlmTestResponse),
        (status = 400, description = "Invalid payload or model not found")
    )
)]
pub(crate) async fn test_project_llm_model_handler(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
    Json(req): Json<llm_probe::LlmTestRequest>,
) -> Result<Json<llm_probe::LlmTestResponse>, ApiError> {
    let resp = llm_probe::probe_project_llm_model(&state.session_db, proj_id, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(resp))
}

#[utoipa::path(
    delete,
    path = "/v1/projects/{proj_id}/inference/llm-models/{model_id}",
    tag = "ProjectInference",
    operation_id = "delete_project_llm_model_handler",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("model_id" = String, Path, description = "LLM model id")
    ),
    responses(
        (status = 204, description = "LLM model deleted"),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "LLM model not found")
    )
)]
pub(crate) async fn delete_project_llm_model_handler(
    State(state): State<AppState>,
    AxumPath((proj_id, model_id)): AxumPath<(i64, String)>,
) -> Result<StatusCode, ApiError> {
    let (deleted, inherit_now) =
        gateway_project_llm::delete_project_llm_model(&state.session_db, proj_id, &model_id)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if !deleted {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "llm model not found"));
    }
    if inherit_now {
        if let Some(client) = state.pool_clients.e2b_sandbox_client() {
            let _ = gateway_project_observe::teardown_project_observe(
                &state.session_db,
                client,
                proj_id,
            )
            .await;
        } else if let Some(cluster_id) =
            crate::gateway_llm_cluster_store::resolve_llm_cluster_id()
        {
            let _ = state
                .session_db
                .delete_llm_project_observe(&cluster_id, proj_id)
                .await;
        }
    } else if let Some(client) = state.pool_clients.e2b_sandbox_client() {
        let _ = gateway_project_observe::ensure_project_observe(&state.session_db, client, proj_id)
            .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/inference/llm-models/{model_id}/versions",
    tag = "ProjectInference",
    operation_id = "list_project_llm_model_versions_handler",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("model_id" = String, Path, description = "LLM model id")
    ),
    responses(
        (status = 200, description = "LLM model revision list", body = gateway_global_settings::LlmModelVersionsResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub(crate) async fn list_project_llm_model_versions_handler(
    State(state): State<AppState>,
    AxumPath((proj_id, model_id)): AxumPath<(i64, String)>,
) -> Result<Json<gateway_global_settings::LlmModelVersionsResponse>, ApiError> {
    let body =
        gateway_project_llm::list_project_llm_model_versions(&state.session_db, proj_id, &model_id)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/projects/{proj_id}/inference/llm-models/{model_id}/apply",
    tag = "ProjectInference",
    operation_id = "apply_project_llm_model_head_handler",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("model_id" = String, Path, description = "LLM model id")
    ),
    responses(
        (status = 200, description = "LLM model head applied", body = gateway_global_settings::ApplyLlmModelResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub(crate) async fn apply_project_llm_model_head_handler(
    State(state): State<AppState>,
    AxumPath((proj_id, model_id)): AxumPath<(i64, String)>,
) -> Result<Json<gateway_global_settings::ApplyLlmModelResponse>, ApiError> {
    let resp = gateway_project_llm::apply_project_llm_model_by_id(
        &state.session_db,
        proj_id,
        &model_id,
        None,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if let Some(client) = state.pool_clients.e2b_sandbox_client() {
        let _ = gateway_project_observe::ensure_project_observe(&state.session_db, client, proj_id)
            .await;
    }
    Ok(Json(resp))
}

#[utoipa::path(
    post,
    path = "/v1/projects/{proj_id}/inference/llm-models/{model_id}/versions/{model_rev}/apply",
    tag = "ProjectInference",
    operation_id = "apply_project_llm_model_revision_handler",
    params(
        ("proj_id" = i64, Path, description = "Project ID"),
        ("model_id" = String, Path, description = "LLM model id"),
        ("model_rev" = String, Path, description = "LLM model revision id")
    ),
    responses(
        (status = 200, description = "LLM model revision applied", body = gateway_global_settings::ApplyLlmModelResponse),
        (status = 400, description = "Invalid request")
    )
)]
pub(crate) async fn apply_project_llm_model_revision_handler(
    State(state): State<AppState>,
    AxumPath((proj_id, model_id, model_rev)): AxumPath<(i64, String, String)>,
) -> Result<Json<gateway_global_settings::ApplyLlmModelResponse>, ApiError> {
    let resp = gateway_project_llm::apply_project_llm_model_by_id(
        &state.session_db,
        proj_id,
        &model_id,
        Some(&model_rev),
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if let Some(client) = state.pool_clients.e2b_sandbox_client() {
        let _ = gateway_project_observe::ensure_project_observe(&state.session_db, client, proj_id)
            .await;
    }
    Ok(Json(resp))
}

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/inference/observe",
    tag = "ProjectInference",
    operation_id = "get_project_observe_handler",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Observe sandbox status", body = gateway_project_observe::ProjectObserveStatusResponse),
        (status = 400, description = "Invalid projId")
    )
)]
pub(crate) async fn get_project_observe_handler(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<gateway_project_observe::ProjectObserveStatusResponse>, ApiError> {
    let body = gateway_project_observe::get_project_observe_status(
        &state.session_db,
        state.pool_clients.e2b_sandbox_client().map(|v| &**v),
        proj_id,
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    post,
    path = "/v1/projects/{proj_id}/inference/observe/reset",
    tag = "ProjectInference",
    operation_id = "reset_project_observe_handler",
    params(("proj_id" = i64, Path, description = "Project ID")),
    responses(
        (status = 200, description = "Observe sandbox reset", body = gateway_project_observe::ProjectObserveResetResponse),
        (status = 503, description = "E2B sandbox client not configured"),
        (status = 502, description = "Upstream E2B error")
    )
)]
pub(crate) async fn reset_project_observe_handler(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<gateway_project_observe::ProjectObserveResetResponse>, ApiError> {
    let client = state.pool_clients.e2b_sandbox_client().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "e2b sandbox client not configured",
        )
    })?;
    let body = gateway_project_observe::reset_project_observe(&state.session_db, client, proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    Ok(Json(body))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub(crate) struct ResetProjectE2bWorkerQuery {
    #[serde(rename = "slotIndex")]
    slot_index: Option<u32>,
}

