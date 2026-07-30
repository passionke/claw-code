//! projects routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::*;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/projects", get(list_projects).post(create_project))
        .route(
            "/v1/projects/{proj_id}",
            patch(patch_project).delete(delete_project),
        )
        .route("/v1/projects/{proj_id}/git/pull", post(pull_project_git))
        .route("/v1/init", post(init_workspace))
}
