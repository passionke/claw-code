//! admin_mcp routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::*;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/v1/admin/mcp", post(admin_mcp_http_handler))
}
