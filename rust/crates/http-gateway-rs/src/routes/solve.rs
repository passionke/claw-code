//! solve routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{solve, solve_async, solve_start};
use axum::routing::post;
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/solve", post(solve))
        .route("/v1/start", post(solve_start))
        .route("/v1/solve_async", post(solve_async))
}
