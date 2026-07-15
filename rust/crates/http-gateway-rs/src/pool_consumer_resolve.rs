//! Pool v1: resolve worker artifacts for HTTP consumers from PostgreSQL only.
//! Author: kejiqing
//!
//! `running` turns: e2b pool syncs NAS session `.claw` progress into PG via nas-api
//! (`sync_turn_progress_to_db`). See `docs/pool-v1-consumer-matrix.md`.

use std::sync::Arc;

use gateway_solve_turn::multi_agent::SolveTurnTimeline;
use gateway_solve_turn::{ProgressEvent, TaskProgressFile};

use crate::pool::PoolOps;
use crate::session_db::GatewaySessionDb;
use crate::turn_timeline_api;

#[derive(Debug, Clone, Default)]
pub struct TurnProgressSnapshot {
    pub events: Vec<ProgressEvent>,
    pub task_progress: Option<TaskProgressFile>,
}

/// Running turns: pool pulls session `.claw` (nas-api) and upserts PG before consumer read.
pub async fn maybe_sync_running_turn_progress_from_worker(
    pool: &Arc<dyn PoolOps + Send + Sync>,
    turn_id: &str,
    status: &str,
) {
    if status != "running" || turn_id.is_empty() {
        return;
    }
    let _ = pool.sync_turn_progress_to_db(turn_id).await;
}

pub async fn resolve_turn_progress(
    db: &GatewaySessionDb,
    turn_id: &str,
    event_limit: usize,
) -> Result<TurnProgressSnapshot, String> {
    let mut snap = TurnProgressSnapshot::default();
    if turn_id.is_empty() {
        return Ok(snap);
    }
    if let Ok(Some(store)) = db.get_turn_solve_timing_json(turn_id).await {
        snap.events = GatewaySessionDb::progress_events_from_timing_store(&store, event_limit);
        snap.task_progress = GatewaySessionDb::task_progress_from_timing_store(&store);
    }
    Ok(snap)
}

/// Timeline swimlane from `gateway_turns.solve_timing_jsonb`.
pub async fn resolve_turn_timeline(
    db: &GatewaySessionDb,
    turn_id: &str,
    created_at_ms: i64,
    finished_at_ms: Option<i64>,
) -> Result<Option<SolveTurnTimeline>, String> {
    if turn_id.is_empty() {
        return Ok(None);
    }
    let Ok(Some(store)) = db.get_turn_solve_timing_json(turn_id).await else {
        return Ok(None);
    };
    Ok(turn_timeline_api::load_turn_timeline_from_db(
        &store,
        created_at_ms,
        finished_at_ms,
    ))
}

#[must_use]
pub fn plan_fields_from_snapshot(
    snap: &TurnProgressSnapshot,
) -> (Option<String>, Vec<gateway_solve_turn::TaskProgressTodo>) {
    let Some(p) = snap.task_progress.as_ref() else {
        return (None, Vec::new());
    };
    (p.plan_title.clone(), p.todos.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_db::{connect_gateway_test_db, GatewaySessionDb};
    use gateway_solve_turn::{
        progress_events_path, record_mcp_tool_started, run_report_progress,
        task_progress_json_path, ProgressEvent, TaskProgressFile,
    };
    use serde_json::json;
    use std::fs;

    /// Worker writes `.claw/*` the same way resolve/sync later ingest — keep serde closed. Author: kejiqing
    fn worker_write_progress_fixture(session_home: &std::path::Path, session_id: &str) {
        fs::create_dir_all(session_home).unwrap();
        let input = json!({
            "current_task_desc": "分析框架已生成",
            "phase": "planned",
            "plan_title": "近7天经营分析",
            "todos": [
                {"id": "1", "title": "拉每日销售额", "status": "pending"},
                {"id": "2", "title": "看支付结构", "status": "pending"}
            ],
            "current_todo_id": null
        });
        run_report_progress(session_home, session_id, &input).unwrap();
        let mcp_args = json!({
            "question": "Wanghin Branch近7天销售额是多少？"
        });
        record_mcp_tool_started(session_home, session_id, None, &mcp_args).unwrap();
        let progress_after = json!({
            "current_task_desc": "正在查询：拉每日销售额（0/2）",
            "phase": "executing_todo",
            "plan_title": "近7天经营分析",
            "todos": [
                {"id": "1", "title": "拉每日销售额", "status": "in_progress"},
                {"id": "2", "title": "看支付结构", "status": "pending"}
            ],
            "current_todo_id": "1"
        });
        run_report_progress(session_home, session_id, &progress_after).unwrap();
    }

    /// Same parsing rules as [`GatewaySessionDb::replace_turn_progress_snapshot`] without PG. Author: kejiqing
    fn parse_like_replace_turn_progress_snapshot(
        progress_ndjson: &str,
        task_progress_json: &str,
    ) -> (Vec<ProgressEvent>, Option<TaskProgressFile>) {
        let mut events = Vec::new();
        for line in progress_ndjson.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let ev: ProgressEvent =
                serde_json::from_str(line).expect("worker ProgressEvent ndjson must deserialize");
            events.push(ev);
        }
        let task = if task_progress_json.trim().is_empty() {
            None
        } else {
            Some(
                serde_json::from_str::<TaskProgressFile>(task_progress_json)
                    .expect("worker task-progress.json must deserialize as TaskProgressFile"),
            )
        };
        (events, task)
    }

    #[test]
    fn worker_claw_progress_files_parse_like_gateway_replace_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let session_home = dir.path().join("sess");
        let session_id = "sess-contract-1";
        worker_write_progress_fixture(&session_home, session_id);

        let progress_ndjson = fs::read_to_string(progress_events_path(&session_home)).unwrap();
        let task_progress_json =
            fs::read_to_string(task_progress_json_path(&session_home)).unwrap();

        assert!(
            progress_ndjson.contains("report_progress"),
            "ndjson should include report_progress kind from tool writes"
        );
        assert!(
            progress_ndjson.contains("mcp_tool_started"),
            "ndjson should include mcp_tool_started from MCP progress hook"
        );

        let (events, task) =
            parse_like_replace_turn_progress_snapshot(&progress_ndjson, &task_progress_json);
        assert!(
            events.len() >= 2,
            "expected report_progress + mcp started (+ more)"
        );
        assert!(events.iter().any(|e| e.kind == "report_progress"));
        assert!(events.iter().any(|e| e.kind == "mcp_tool_started"));
        assert!(
            events
                .iter()
                .any(|e| e.message.contains("Wanghin") || e.message.contains("销售额")),
            "mcp question should surface as progress message: {events:?}"
        );

        let task = task.expect("task-progress.json");
        assert_eq!(task.session_id, session_id);
        assert_eq!(task.plan_title.as_deref(), Some("近7天经营分析"));
        assert_eq!(task.todos.len(), 2);
        assert_eq!(task.todos[0].status, "in_progress");
        assert_eq!(task.current_todo_id.as_deref(), Some("1"));
        assert!(task.current_task_desc.contains("拉每日销售额"));

        let store = json!({
            "progressEvents": events,
            "taskProgress": task,
        });
        let from_store = GatewaySessionDb::progress_events_from_timing_store(&store, 50);
        assert_eq!(from_store.len(), events.len());
        let tp = GatewaySessionDb::task_progress_from_timing_store(&store).unwrap();
        assert_eq!(tp.todos[0].title, "拉每日销售额");
        let snap = TurnProgressSnapshot {
            events: from_store,
            task_progress: Some(tp),
        };
        let (plan, todos) = plan_fields_from_snapshot(&snap);
        assert_eq!(plan.as_deref(), Some("近7天经营分析"));
        assert_eq!(todos.len(), 2);
    }

    #[tokio::test]
    async fn worker_claw_progress_files_roundtrip_through_replace_and_resolve() {
        let Some(db) = connect_gateway_test_db().await else {
            eprintln!(
                "skip worker_claw_progress_files_roundtrip_through_replace_and_resolve: \
                 set CLAW_GATEWAY_TEST_DATABASE_URL"
            );
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let session_home = dir.path().join("sess");
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let session_id = format!("contract_{uid}");
        let turn_id = format!("T_{uid}");
        worker_write_progress_fixture(&session_home, &session_id);

        let progress_ndjson = fs::read_to_string(progress_events_path(&session_home)).unwrap();
        let task_progress_json =
            fs::read_to_string(task_progress_json_path(&session_home)).unwrap();

        let t = crate::persistence::transcript::now_ms();
        db.insert_session(&session_id, 1, "proj_1/sessions/none", t, None)
            .await
            .unwrap();
        db.insert_turn(
            &turn_id,
            &session_id,
            1,
            "running",
            t,
            Some("q"),
            None,
            None,
        )
        .await
        .unwrap();
        db.replace_turn_progress_snapshot(&turn_id, &progress_ndjson, &task_progress_json)
            .await
            .unwrap();

        let snap = resolve_turn_progress(&db, &turn_id, 50).await.unwrap();
        assert!(snap.events.len() >= 2);
        assert!(snap.events.iter().any(|e| e.kind == "report_progress"));
        assert!(snap.events.iter().any(|e| e.kind == "mcp_tool_started"));
        let (plan, todos) = plan_fields_from_snapshot(&snap);
        assert_eq!(plan.as_deref(), Some("近7天经营分析"));
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].status, "in_progress");
        let desc = snap
            .task_progress
            .as_ref()
            .map(|p| p.current_task_desc.as_str())
            .unwrap_or("");
        assert!(desc.contains("拉每日销售额"), "currentTaskDesc={desc}");
    }

    #[tokio::test]
    async fn resolve_turn_progress_reads_pg_without_disk_jsonl() {
        let Some(db) = connect_gateway_test_db().await else {
            eprintln!("skip resolve_turn_progress_reads_pg_without_disk_jsonl: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let turn_id = format!("T_{uid}");
        let sid = format!("prog_{uid}");
        let t = crate::persistence::transcript::now_ms();
        db.insert_session(&sid, 1, "proj_1/sessions/none", t, None)
            .await
            .unwrap();
        db.insert_turn(&turn_id, &sid, 1, "running", t, Some("q"), None, None)
            .await
            .unwrap();
        let progress_ndjson = r#"{"kind":"report_progress","message":"查表中","tsMs":1000}
{"kind":"mcp_tool_started","message":"sql","tsMs":2000}
"#;
        let task_json = json!({
            "version": 1,
            "sessionId": sid,
            "currentTaskDesc": "分析门店",
            "phase": "query",
            "planTitle": "经营分析",
            "todos": [{"id":"1","title":"拉数","status":"pending"}],
            "updatedAtMs": 1000
        })
        .to_string();
        db.replace_turn_progress_snapshot(&turn_id, progress_ndjson, &task_json)
            .await
            .unwrap();

        let snap = resolve_turn_progress(&db, &turn_id, 50).await.unwrap();
        assert_eq!(snap.events.len(), 2);
        assert_eq!(snap.events[0].message, "查表中");
        let (plan, todos) = plan_fields_from_snapshot(&snap);
        assert_eq!(plan.as_deref(), Some("经营分析"));
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].title, "拉数");
    }

    #[tokio::test]
    async fn resolve_turn_timeline_reads_pg_without_disk() {
        let Some(db) = connect_gateway_test_db().await else {
            eprintln!("skip resolve_turn_timeline_reads_pg_without_disk: set CLAW_GATEWAY_TEST_DATABASE_URL");
            return;
        };
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let turn_id = format!("T_{uid}");
        let sid = format!("tl_{uid}");
        let t0 = 1_000_000_i64;
        db.insert_session(&sid, 1, "proj_1/sessions/none", t0, None)
            .await
            .unwrap();
        db.insert_turn(&turn_id, &sid, 1, "succeeded", t0, Some("q"), None, None)
            .await
            .unwrap();
        let store = json!({
            "solveTimingEvents": [
                {"kind":"llm_iter","tsMs": t0 + 1000, "turnId": turn_id}
            ],
            "orchestrationEvents": [],
            "progressEvents": [
                {"kind":"report_progress","message":"大纲就绪","tsMs": t0 + 500}
            ],
            "taskProgress": null
        });
        db.upsert_turn_timing_json(&turn_id, &store).await.unwrap();

        let tl = resolve_turn_timeline(&db, &turn_id, t0, Some(t0 + 5000))
            .await
            .unwrap()
            .expect("timeline from PG");
        assert!(tl.lanes.iter().any(|l| !l.segments.is_empty()));
    }
}
