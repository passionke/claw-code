// Fragment of routes::app (include!). Router delegate HTTP. Author: kejiqing

#[utoipa::path(
    get,
    path = "/v1/projects/{proj_id}/delegate-targets",
    tag = "Router",
    operation_id = "get_delegate_targets",
    params(("proj_id" = i64, Path, description = "Initiator project id")),
    responses((status = 200, body = crate::delegate_router::DelegateTargetsResponse))
)]
pub(crate) async fn get_delegate_targets(
    State(state): State<AppState>,
    AxumPath(proj_id): AxumPath<i64>,
) -> Result<Json<crate::delegate_router::DelegateTargetsResponse>, ApiError> {
    ensure_delegate_initiator(&state, proj_id).await?;
    let targets = state
        .session_db
        .list_delegate_targets(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(crate::delegate_router::DelegateTargetsResponse {
        initiator_proj_id: proj_id,
        targets,
    }))
}

#[utoipa::path(
    put,
    path = "/v1/projects/{proj_id}/delegate-targets",
    tag = "Router",
    operation_id = "put_delegate_targets",
    params(("proj_id" = i64, Path)),
    request_body = crate::delegate_router::PutDelegateTargetsRequest,
    responses((status = 200, body = crate::delegate_router::DelegateTargetsResponse))
)]
pub(crate) async fn put_delegate_targets(
    State(state): State<AppState>,
    AxumPath(initiator_proj_id): AxumPath<i64>,
    Json(req): Json<crate::delegate_router::PutDelegateTargetsRequest>,
) -> Result<Json<crate::delegate_router::DelegateTargetsResponse>, ApiError> {
    ensure_delegate_initiator(&state, initiator_proj_id).await?;
    for spec in &req.targets {
        if spec.target_proj_id == initiator_proj_id {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "cannot delegate to self",
            ));
        }
        let role = state
            .session_db
            .get_project_role(spec.target_proj_id)
            .await
            .map_err(|e| session_db_err(&e))?;
        if role != master_observer::PROJECT_ROLE_NORMAL {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "target {} must be project_role=normal (got {role})",
                    spec.target_proj_id
                ),
            ));
        }
        state
            .session_db
            .get_project_config(spec.target_proj_id)
            .await
            .map_err(|e| session_db_err(&e))?
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("project_config missing for target {}", spec.target_proj_id),
                )
            })?;
    }
    state
        .session_db
        .replace_delegate_targets(initiator_proj_id, &req.targets)
        .await
        .map_err(|e| session_db_err(&e))?;
    let targets = state
        .session_db
        .list_delegate_targets(initiator_proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(crate::delegate_router::DelegateTargetsResponse {
        initiator_proj_id,
        targets,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/projects/{proj_id}/delegate/resolve-session",
    tag = "Router",
    operation_id = "resolve_delegate_session",
    params(("proj_id" = i64, Path)),
    request_body = crate::delegate_router::ResolveDelegateSessionRequest,
    responses((status = 200, body = crate::delegate_router::ResolveDelegateSessionResponse))
)]
pub(crate) async fn resolve_delegate_session(
    State(state): State<AppState>,
    AxumPath(initiator_proj_id): AxumPath<i64>,
    Json(req): Json<crate::delegate_router::ResolveDelegateSessionRequest>,
) -> Result<Json<crate::delegate_router::ResolveDelegateSessionResponse>, ApiError> {
    ensure_delegate_initiator(&state, initiator_proj_id).await?;
    if req.parent_session_id.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "parentSessionId required",
        ));
    }
    state
        .session_db
        .assert_delegate_target_allowed(initiator_proj_id, req.delegate_proj_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    let (delegate_session_id, root_session_id, created) = state
        .session_db
        .resolve_or_create_delegate_session(
            initiator_proj_id,
            req.parent_session_id.trim(),
            req.delegate_proj_id,
            Some("delegate_project"),
        )
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(
        crate::delegate_router::ResolveDelegateSessionResponse {
            delegate_session_id,
            root_session_id,
            created,
        },
    ))
}

async fn ensure_delegate_initiator(state: &AppState, proj_id: i64) -> Result<(), ApiError> {
    let role = state
        .session_db
        .get_project_role(proj_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if role != master_observer::PROJECT_ROLE_ROUTER && role != master_observer::PROJECT_ROLE_NORMAL {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("project {proj_id} cannot initiate delegate (role={role})"),
        ));
    }
    Ok(())
}
