//! Master / observation pairing and schedule HTTP routes. Author: kejiqing

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::app_state::AppState;
use crate::routes::app::{
    delete_master_schedule, get_master_apprentices, get_master_repair_run, list_master_repair_runs,
    list_master_schedules, master_mcp_http_handler, master_peer_create_observation,
    master_peer_observation_draft, master_peer_observation_solve, master_peer_put_draft,
    master_peer_replay_turn, master_peer_session_turns, master_peer_sessions,
    master_peer_stable_config, master_peer_sync_observation, put_master_apprentices,
    put_master_role, put_master_schedule, run_master_schedule,
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
        .route(
            "/v1/master-peer/projects/{proj_id}/stable-config",
            get(master_peer_stable_config),
        )
        .route(
            "/v1/master-peer/projects/{proj_id}/sessions",
            get(master_peer_sessions),
        )
        .route(
            "/v1/master-peer/projects/{proj_id}/sessions/{session_id}/turns",
            get(master_peer_session_turns),
        )
        .route(
            "/v1/master-peer/projects/{proj_id}/replay-turn",
            get(master_peer_replay_turn),
        )
        .route(
            "/v1/master-peer/projects/{proj_id}/draft",
            put(master_peer_put_draft),
        )
        .route(
            "/v1/master-peer/projects/{proj_id}/observation",
            post(master_peer_create_observation),
        )
        .route(
            "/v1/master-peer/observations/{observation_proj_id}/sync-from/{apprentice_proj_id}",
            post(master_peer_sync_observation),
        )
        .route(
            "/v1/master-peer/observations/{observation_proj_id}/draft",
            put(master_peer_observation_draft),
        )
        .route(
            "/v1/master-peer/observations/{observation_proj_id}/solve",
            post(master_peer_observation_solve),
        )
}
