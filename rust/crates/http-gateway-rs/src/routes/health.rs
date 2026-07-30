//! health routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{healthz, readyz};
use axum::routing::get;
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
}
