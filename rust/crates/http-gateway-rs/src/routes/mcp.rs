//! mcp routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::*;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/mcp/inject", post(inject_mcp))
        .route(
            "/v1/mcp/injected/{proj_id}",
            get(get_injected_mcp).delete(delete_injected_mcp),
        )
        .route("/v1/mcp/test", post(test_mcp))
}
