// Fragment of routes::app (include!). Author: kejiqing

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct DeleteGatewayEndpointResponse {
    #[serde(rename = "gatewayId")]
    gateway_id: String,
    deleted: bool,
}

#[utoipa::path(
    get,
    path = "/v1/preflight/plugins",
    tag = "Pools",
    operation_id = "get_preflight_plugins_handler",
    responses(
        (status = 200, description = "Global preflight plugin registry", body = preflight_plugin_api::PreflightPluginListResponse)
    )
)]
pub(crate) async fn get_preflight_plugins_handler(
    State(state): State<AppState>,
) -> Result<Json<preflight_plugin_api::PreflightPluginListResponse>, ApiError> {
    preflight_plugin_api::list_preflight_plugins(&state.session_db)
        .await
        .map_err(|(status, msg)| ApiError::new(status, msg))
        .map(Json)
}

#[utoipa::path(
    put,
    path = "/v1/preflight/plugins/{plugin_id}",
    tag = "Pools",
    operation_id = "put_preflight_plugin_handler",
    params(
        ("plugin_id" = String, Path, description = "Plugin id")
    ),
    request_body = preflight_plugin_api::UpsertPreflightPluginRequest,
    responses(
        (status = 200, description = "Plugin upserted", body = Object),
        (status = 400, description = "Invalid plugin payload")
    )
)]
pub(crate) async fn put_preflight_plugin_handler(
    State(state): State<AppState>,
    AxumPath(plugin_id): AxumPath<String>,
    Json(req): Json<preflight_plugin_api::UpsertPreflightPluginRequest>,
) -> Result<Json<preflight_spi::PreflightPluginRecord>, ApiError> {
    preflight_plugin_api::upsert_preflight_plugin(&state.session_db, &plugin_id, req)
        .await
        .map_err(|(status, msg)| ApiError::new(status, msg))
        .map(Json)
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ListClawPoolsResponse {
    pools: Vec<ClawPoolJson>,
    #[serde(rename = "coLocatedPoolId", skip_serializing_if = "Option::is_none")]
    co_located_pool_id: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct ClawPoolJson {
    #[serde(rename = "poolId")]
    pool_id: String,
    #[serde(rename = "advertiseIp")]
    advertise_ip: String,
    #[serde(rename = "ssePort")]
    sse_port: i32,
    #[serde(rename = "slotsMax")]
    slots_max: i32,
    #[serde(rename = "slotsMin")]
    slots_min: i32,
    #[serde(rename = "registrationTimeMs")]
    registration_time_ms: i64,
    #[serde(rename = "lastHeartbeatMs")]
    last_heartbeat_ms: i64,
    online: bool,
    #[serde(rename = "httpBase")]
    http_base: String,
    #[serde(rename = "gatewayBase", skip_serializing_if = "String::is_empty")]
    gateway_base: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct DeleteClawPoolResponse {
    #[serde(rename = "poolId")]
    pool_id: String,
    deleted: bool,
}

#[utoipa::path(
    get,
    path = "/v1/gateway/endpoints",
    tag = "Pools",
    operation_id = "list_gateway_endpoints_handler",
    responses(
        (status = 200, description = "Registered gateway endpoints in cluster", body = crate::gateway_endpoint::GatewayEndpointsResponse)
    )
)]
pub(crate) async fn list_gateway_endpoints_handler(
    State(state): State<AppState>,
) -> Result<Json<crate::gateway_endpoint::GatewayEndpointsResponse>, ApiError> {
    let body = crate::gateway_endpoint::list_endpoints_response(
        &state.session_db,
        state.gateway_identity.as_ref(),
    )
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(body))
}

#[utoipa::path(
    delete,
    path = "/v1/gateway/endpoints/{gateway_id}",
    tag = "Pools",
    operation_id = "delete_gateway_endpoint_handler",
    params(
        ("gateway_id" = String, Path, description = "Gateway id to remove from registry")
    ),
    responses(
        (status = 200, description = "Delete result", body = DeleteGatewayEndpointResponse),
        (status = 400, description = "Cannot delete self or empty gateway_id")
    )
)]
pub(crate) async fn delete_gateway_endpoint_handler(
    State(state): State<AppState>,
    AxumPath(gateway_id): AxumPath<String>,
) -> Result<Json<DeleteGatewayEndpointResponse>, ApiError> {
    let gid = gateway_id.trim();
    if gid.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "gateway_id must not be empty",
        ));
    }
    if gid == state.gateway_identity.gateway_id {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cannot delete self gateway endpoint while process is running",
        ));
    }
    let deleted = state
        .session_db
        .delete_gateway_endpoint(gid)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(DeleteGatewayEndpointResponse {
        gateway_id: gid.to_string(),
        deleted,
    }))
}

#[utoipa::path(
    delete,
    path = "/v1/pools/{pool_id}",
    tag = "Pools",
    operation_id = "delete_claw_pool_handler",
    params(
        ("pool_id" = String, Path, description = "Claw pool id")
    ),
    responses(
        (status = 200, description = "Pool deleted", body = DeleteClawPoolResponse),
        (status = 404, description = "Pool not found")
    )
)]
pub(crate) async fn delete_claw_pool_handler(
    State(state): State<AppState>,
    AxumPath(pool_id): AxumPath<String>,
) -> Result<Json<DeleteClawPoolResponse>, ApiError> {
    let pool_id = pool_id.trim();
    if pool_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "poolId must not be empty",
        ));
    }
    let deleted = state
        .session_db
        .delete_claw_pool(pool_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if !deleted {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("claw_pool row not found: {pool_id}"),
        ));
    }
    Ok(Json(DeleteClawPoolResponse {
        pool_id: pool_id.to_string(),
        deleted: true,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/pools",
    tag = "Pools",
    operation_id = "list_claw_pools_handler",
    responses(
        (status = 200, description = "Registered claw pools", body = ListClawPoolsResponse)
    )
)]
pub(crate) async fn list_claw_pools_handler(
    State(state): State<AppState>,
) -> Result<Json<ListClawPoolsResponse>, ApiError> {
    let rows = state
        .session_db
        .list_claw_pools()
        .await
        .map_err(|e| session_db_err(&e))?;
    let now = session_db::now_ms_for_registry();
    let pools = rows
        .into_iter()
        .map(|r| {
            let online = session_db::is_claw_pool_online(r.last_heartbeat_ms, now);
            ClawPoolJson {
                pool_id: r.pool_id.clone(),
                advertise_ip: r.advertise_ip.clone(),
                sse_port: r.sse_port,
                slots_max: r.slots_max,
                slots_min: r.slots_min,
                registration_time_ms: r.registration_time_ms,
                last_heartbeat_ms: r.last_heartbeat_ms,
                online,
                http_base: format!("http://{}:{}", r.advertise_ip, r.sse_port),
                gateway_base: r.gateway_base.clone(),
            }
        })
        .collect();
    Ok(Json(ListClawPoolsResponse {
        pools,
        co_located_pool_id: Some(state.pool_clients.pool_id().to_string()),
    }))
}

