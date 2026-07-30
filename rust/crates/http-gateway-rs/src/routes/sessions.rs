//! sessions routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::*;
use crate::session_upload;
use axum::routing::{delete, get, patch, post, put};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/projects/{proj_id}/sessions",
            get(list_project_sessions),
        )
        .route(
            "/v1/sessions/{session_id}/execution",
            get(get_session_execution),
        )
        .route(
            "/v1/sessions/{session_id}/files",
            post(session_upload::upload_session_files),
        )
        .route(
            "/v1/sessions/{session_id}/conversation_translate",
            get(get_conversation_translate).post(rebuild_conversation_translate),
        )
        .route("/v1/sessions/{session_id}/agent/ws", get(agent_ws_handler))
        .route(
            "/v1/projects/{proj_id}/ovs/workspace",
            get(ovs_workspace_handler),
        )
        .route("/v1/gateway/translate", post(post_gateway_translate))
}
