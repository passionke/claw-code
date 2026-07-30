//! tasks routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{cancel_task, get_task};
use axum::routing::{get, post};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/tasks/{task_id}", get(get_task))
        .route("/v1/tasks/{task_id}/cancel", post(cancel_task))
}
