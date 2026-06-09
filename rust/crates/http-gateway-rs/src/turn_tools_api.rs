//! `GET /v1/sessions/{session_id}/turns/{turn_id}/tools`. Author: kejiqing

use gateway_solve_turn::turn_tools::{
    list_tool_executions_for_user_turn_from_jsonl_with_time_window, TurnToolRecord,
};
use gateway_solve_turn::ProgressEvent;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnToolsResponse {
    pub session_id: String,
    pub turn_id: String,
    pub proj_id: i64,
    pub user_turn_index: i64,
    pub tools: Vec<TurnToolRecord>,
}

/// Tools for one user turn from PG transcript + PG progress events (pool v1 consumer path).
pub fn list_turn_tools_for_session(
    session_jsonl_from_db: &str,
    progress_events: &[ProgressEvent],
    user_turn_index_1based: usize,
    turn_created_at_ms: Option<i64>,
    turn_finished_at_ms: Option<i64>,
) -> Result<Vec<TurnToolRecord>, String> {
    list_tool_executions_for_user_turn_from_jsonl_with_time_window(
        session_jsonl_from_db,
        progress_events,
        user_turn_index_1based,
        turn_created_at_ms,
        turn_finished_at_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::transcript::{import_turn_messages_to_db, JsonlMessage};
    use crate::session_db::connect_gateway_test_db;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0_i64, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    #[test]
    fn list_turn_tools_from_pg_jsonl_only() {
        let pg_jsonl = r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"ping"}]}}
{"type":"message","message":{"role":"assistant","blocks":[{"type":"tool_use","id":"tu1","name":"mcp__sqlbot","input":{}}]}}
{"type":"message","message":{"role":"user","blocks":[{"type":"tool_result","tool_use_id":"tu1","output":"ok","is_error":false}]}}
"#;
        let tools = list_turn_tools_for_session(pg_jsonl, &[], 1, Some(1000), Some(2000)).unwrap();
        assert_eq!(tools.len(), 1, "expected tool from PG jsonl");
        assert_eq!(tools[0].name, "mcp__sqlbot");
        assert_eq!(tools[0].output.as_deref(), Some("ok"));
    }

    #[test]
    fn list_turn_tools_empty_when_pg_jsonl_empty() {
        let tools = list_turn_tools_for_session("", &[], 1, None, None).unwrap();
        assert!(tools.is_empty(), "empty PG transcript must yield no tools");
    }

    /// End-to-end: import_turn_messages → render_session_jsonl → tools API.
    #[tokio::test]
    async fn tools_api_chain_pg_transcript_without_disk_jsonl() {
        let Some(db) = connect_gateway_test_db().await else {
            eprintln!(
                "skip tools_api_chain_pg_transcript_without_disk_jsonl: set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let t = now_ms();
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let sid = format!("tools_chain_{uid}");
        let turn_id = format!("T_{uid}");
        let session_home_rel = format!("proj_1/sessions/{sid}");
        db.insert_session(&sid, 1, &session_home_rel, t, None)
            .await
            .unwrap();
        db.insert_turn(
            &turn_id,
            &sid,
            1,
            "queued",
            t,
            Some("list tables"),
            None,
            None,
        )
        .await
        .unwrap();

        let messages = vec![
            JsonlMessage {
                role: "user".to_string(),
                blocks: json!([{"type":"text","text":"list tables"}]),
                usage: None,
            },
            JsonlMessage {
                role: "assistant".to_string(),
                blocks: json!([{"type":"tool_use","id":"mcp1","name":"mcp__sqlbot-streamable__query","input":{"sql":"select 1"}}]),
                usage: None,
            },
            JsonlMessage {
                role: "user".to_string(),
                blocks: json!([{"type":"tool_result","tool_use_id":"mcp1","output":"[[1]]","is_error":false}]),
                usage: None,
            },
        ];
        import_turn_messages_to_db(&db, &sid, 1, &turn_id, &messages, t)
            .await
            .unwrap();
        db.finalize_turn_terminal(
            &turn_id,
            "succeeded",
            Some(t + 5000),
            Some("done"),
            None,
            Some(0),
        )
        .await
        .unwrap();

        let pg_jsonl = db.render_session_jsonl(&sid, 1).await.unwrap();
        assert!(
            pg_jsonl.contains("tool_use"),
            "render_session_jsonl must include tool_use blocks"
        );

        let ctx = db
            .get_turn_tools_context(&turn_id, &sid, 1)
            .await
            .unwrap()
            .expect("turn context");
        assert_eq!(ctx.user_turn_index, 1);

        let tools = list_turn_tools_for_session(
            &pg_jsonl,
            &[],
            usize::try_from(ctx.user_turn_index).unwrap_or(1),
            Some(ctx.created_at_ms),
            ctx.finished_at_ms,
        )
        .unwrap();
        assert_eq!(tools.len(), 1, "PG-only chain must surface tools");
        assert_eq!(tools[0].name, "mcp__sqlbot-streamable__query");
        assert_eq!(tools[0].output.as_deref(), Some("[[1]]"));
    }
}
