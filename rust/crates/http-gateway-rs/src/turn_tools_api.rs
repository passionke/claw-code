//! `GET /v1/sessions/{session_id}/turns/{turn_id}/tools`. Author: kejiqing

use gateway_solve_turn::turn_tools::{list_tool_executions_for_user_turn, TurnToolRecord};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnToolsResponse {
    pub session_id: String,
    pub turn_id: String,
    pub ds_id: i64,
    pub user_turn_index: i64,
    pub tools: Vec<TurnToolRecord>,
}

pub fn list_turn_tools_from_session_home(
    session_home: &std::path::Path,
    user_turn_index_1based: usize,
) -> Result<Vec<TurnToolRecord>, String> {
    list_tool_executions_for_user_turn(session_home, user_turn_index_1based)
}
