//! pools routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{
    delete_claw_pool_handler, delete_gateway_endpoint_handler, get_preflight_plugins_handler,
    list_claw_pools_handler, list_gateway_endpoints_handler, put_preflight_plugin_handler,
};
use axum::routing::{delete, get, put};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/pools", get(list_claw_pools_handler))
        .route("/v1/pools/{pool_id}", delete(delete_claw_pool_handler))
        .route("/v1/gateway/endpoints", get(list_gateway_endpoints_handler))
        .route(
            "/v1/gateway/endpoints/{gateway_id}",
            delete(delete_gateway_endpoint_handler),
        )
        .route("/v1/preflight/plugins", get(get_preflight_plugins_handler))
        .route(
            "/v1/preflight/plugins/{plugin_id}",
            put(put_preflight_plugin_handler),
        )
}
