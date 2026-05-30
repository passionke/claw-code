//! `GET /v1/sessions/{session_id}/turns/{turn_id}/timeline`. Author: kejiqing

use std::path::Path;

use gateway_solve_turn::multi_agent::{build_solve_turn_timeline_for_turn, SolveTurnTimeline};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnTimelineResponse {
    pub session_id: String,
    pub turn_id: String,
    pub ds_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_created_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_finished_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeline: Option<SolveTurnTimeline>,
}

pub fn load_turn_timeline(
    session_home: &Path,
    created_at_ms: i64,
    finished_at_ms: Option<i64>,
) -> Option<SolveTurnTimeline> {
    build_solve_turn_timeline_for_turn(session_home, created_at_ms, finished_at_ms)
}
