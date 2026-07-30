//! biz_report routes. Author: kejiqing
use crate::app_state::AppState;
use crate::routes::app::{
    dev_seed_biz_report_task, get_agent_feedback, get_biz_advice_report, get_biz_advice_report_bak,
    post_agent_feedback,
};
use axum::routing::{get, post};
use axum::Router;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/biz_advice_report", get(get_biz_advice_report))
        .route("/v1/biz_advice_report_bak", get(get_biz_advice_report_bak))
        .route(
            "/v1/dev/biz_report_seed_task",
            post(dev_seed_biz_report_task),
        )
        .route(
            "/v1/agent/feedback",
            post(post_agent_feedback).get(get_agent_feedback),
        )
}
