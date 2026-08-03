//! Master / observation pairing and schedule HTTP routes. Author: kejiqing

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::app_state::AppState;
use crate::routes::app::{
    delete_master_schedule, get_master_apprentices, get_master_repair_run, list_master_repair_runs,
    list_master_schedules, master_mcp_http_handler, put_master_apprentices, put_master_role,
    put_master_schedule, run_master_schedule,
};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/projects/{proj_id}/role", put(put_master_role))
        .route(
            "/v1/projects/{proj_id}/apprentices",
            get(get_master_apprentices).put(put_master_apprentices),
        )
        .route(
            "/v1/projects/{proj_id}/repair-runs",
            get(list_master_repair_runs),
        )
        .route(
            "/v1/projects/{proj_id}/repair-runs/{run_id}",
            get(get_master_repair_run),
        )
        .route(
            "/v1/projects/{proj_id}/schedules",
            get(list_master_schedules).put(put_master_schedule),
        )
        .route(
            "/v1/projects/{proj_id}/schedules/{job_id}",
            delete(delete_master_schedule),
        )
        .route(
            "/v1/projects/{proj_id}/schedules/{job_id}/run",
            post(run_master_schedule),
        )
        .route(
            "/v1/master/{master_proj_id}/mcp",
            post(master_mcp_http_handler),
        )
}
