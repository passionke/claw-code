//! Router delegate HTTP routes. Author: kejiqing

use axum::routing::{get, post};
use axum::Router;

use crate::app_state::AppState;
use crate::routes::app::{
    get_delegate_targets, put_delegate_targets, resolve_delegate_session,
};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/v1/projects/{proj_id}/delegate-targets",
            get(get_delegate_targets).put(put_delegate_targets),
        )
        .route(
            "/v1/projects/{proj_id}/delegate/resolve-session",
            post(resolve_delegate_session),
        )
}
