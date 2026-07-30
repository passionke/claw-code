//! project_inference routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::*;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/projects/{proj_id}/e2b-worker",
            get(get_project_e2b_worker_handler),
        )
        .route(
            "/v1/projects/{proj_id}/e2b-worker/reset",
            post(reset_project_e2b_worker_handler),
        )
        .route(
            "/v1/projects/{proj_id}/inference",
            get(get_project_inference_handler),
        )
        .route(
            "/v1/projects/{proj_id}/inference/llm-models",
            post(upsert_project_llm_model_handler),
        )
        .route(
            "/v1/projects/{proj_id}/inference/llm-models/test",
            post(test_project_llm_model_handler),
        )
        .route(
            "/v1/projects/{proj_id}/inference/llm-models/{model_id}",
            delete(delete_project_llm_model_handler),
        )
        .route(
            "/v1/projects/{proj_id}/inference/llm-models/{model_id}/versions",
            get(list_project_llm_model_versions_handler),
        )
        .route(
            "/v1/projects/{proj_id}/inference/llm-models/{model_id}/apply",
            post(apply_project_llm_model_head_handler),
        )
        .route(
            "/v1/projects/{proj_id}/inference/llm-models/{model_id}/versions/{model_rev}/apply",
            post(apply_project_llm_model_revision_handler),
        )
        .route(
            "/v1/projects/{proj_id}/inference/observe",
            get(get_project_observe_handler),
        )
        .route(
            "/v1/projects/{proj_id}/inference/observe/reset",
            post(reset_project_observe_handler),
        )
}
