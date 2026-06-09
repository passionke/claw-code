//! Pool v1: resolve worker artifacts for HTTP consumers from PostgreSQL only.
//! Author: kejiqing
//!
//! `running` turns: pool daemon syncs worker `.claw` progress into PG (`sync_turn_progress_to_db`).
//! See `docs/pool-v1-consumer-matrix.md`.

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

/// Running turns: pool host reads worker `.claw` and upserts PG (gateway container cannot podman exec workers).
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
    use crate::session_db::connect_gateway_test_db;
    use serde_json::json;

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
