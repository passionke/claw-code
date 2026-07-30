//! turns routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{
    cancel_session_turn, get_turn_timeline, get_turn_tools, list_session_turns,
};
use axum::routing::{get, post};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/sessions/{session_id}/turns", get(list_session_turns))
        .route(
            "/v1/sessions/{session_id}/turns/{turn_id}/tools",
            get(get_turn_tools),
        )
        .route(
            "/v1/sessions/{session_id}/turns/{turn_id}/timeline",
            get(get_turn_timeline),
        )
        .route(
            "/v1/sessions/{session_id}/turns/{turn_id}/cancel",
            post(cancel_session_turn),
        )
}
