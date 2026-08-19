//! In-memory hub for worker stdout report deltas (pool-local ingest + live SSE). Author: kejiqing

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::sync::broadcast;

const HUB_CHANNEL_CAP: usize = 4096;

/// Broadcast message: `Delta` for streaming chunks, `SolveDone` as an in-band terminal sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubDeltaChunk {
    pub text: String,
    pub emit_seq: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum HubMsg {
    Delta(HubDeltaChunk),
    SolveDone,
}

#[derive(Debug)]
struct TurnStdoutState {
    text: String,
    chunks: Vec<HubDeltaChunk>,
    has_report: bool,
    solve_done: bool,
    first_report_at_ms: Option<i64>,
    tx: broadcast::Sender<HubMsg>,
}

#[derive(Clone, Default)]
pub struct LiveReportHub {
    inner: std::sync::Arc<Mutex<HashMap<String, TurnStdoutState>>>,
}

impl std::fmt::Debug for LiveReportHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveReportHub").finish_non_exhaustive()
    }
}

impl LiveReportHub {
    pub fn ingest_json(&self, turn_id: &str, value: &Value) {
        let ev = value.get("ev").and_then(Value::as_str).unwrap_or("");
        let mut guard = self.inner.lock().expect("live_report_hub lock");
        let state = guard
            .entry(turn_id.to_string())
            .or_insert_with(|| TurnStdoutState {
                text: String::new(),
                chunks: Vec::new(),
                has_report: false,
                solve_done: false,
                first_report_at_ms: None,
                tx: broadcast::channel(HUB_CHANNEL_CAP).0,
            });
        match ev {
            "report.delta" => {
                let Some(chunk) = value.get("text").and_then(Value::as_str) else {
                    tracing::warn!(
                        target: "claw_live_report",
                        turn_id = %turn_id,
                        "live_report.ingest_skipped — report.delta without text"
                    );
                    return;
                };
                if chunk.is_empty() {
                    return;
                }
                if !state.has_report {
                    state.has_report = true;
                    state.first_report_at_ms = Some(now_ms());
                }
                let emit_seq = value.get("emitSeq").and_then(Value::as_u64);
                state.text.push_str(chunk);
                let delta = HubDeltaChunk {
                    text: chunk.to_string(),
                    emit_seq,
                };
                state.chunks.push(delta.clone());
                let _ = state.tx.send(HubMsg::Delta(delta));
                api::sse_burst_trace::log_pool_ingest(turn_id, chunk, emit_seq);
                crate::biz_report_sse_log::log_stdout_ingest(turn_id, chunk.len());
            }
            "solve.done" => {
                state.solve_done = true;
                let _ = state.tx.send(HubMsg::SolveDone);
                drop(guard);
                self.try_remove_turn(turn_id);
            }
            other => {
                tracing::warn!(
                    target: "claw_live_report",
                    turn_id = %turn_id,
                    ev = %other,
                    "live_report.ingest_unknown_ev"
                );
            }
        }
    }

    /// Ingest one stdout line when prefixed with `__CLAW_GATEWAY_STDOUT__`.
    pub fn ingest_stdout_line(&self, turn_id: &str, line: &str) {
        let Some(value) = gateway_solve_turn::gateway_stdout::parse_stdout_line(line) else {
            crate::live_report_audit::debug_non_claw_stdout_line(turn_id, line);
            return;
        };
        self.ingest_json(turn_id, &value);
    }

    #[must_use]
    pub fn snapshot_text(&self, turn_id: &str) -> String {
        self.inner
            .lock()
            .expect("live_report_hub lock")
            .get(turn_id)
            .map(|s| s.text.clone())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn has_report_for_turn(&self, turn_id: &str) -> bool {
        self.inner
            .lock()
            .expect("live_report_hub lock")
            .get(turn_id)
            .is_some_and(|s| s.has_report)
    }

    #[must_use]
    pub fn is_solve_done(&self, turn_id: &str) -> bool {
        self.inner
            .lock()
            .expect("live_report_hub lock")
            .get(turn_id)
            .is_some_and(|s| s.solve_done)
    }

    #[must_use]
    pub fn first_report_at_ms_for_turn(&self, turn_id: &str) -> Option<i64> {
        self.inner
            .lock()
            .expect("live_report_hub lock")
            .get(turn_id)
            .and_then(|s| s.first_report_at_ms)
    }

    /// Latch report availability onto another turn without fabricating text deltas.
    /// Used by router turns to inherit active specialist report visibility. Author: kejiqing
    pub fn promote_report_availability(&self, turn_id: &str, first_report_at_ms: Option<i64>) {
        let mut guard = self.inner.lock().expect("live_report_hub lock");
        let state = guard
            .entry(turn_id.to_string())
            .or_insert_with(|| TurnStdoutState {
                text: String::new(),
                chunks: Vec::new(),
                has_report: false,
                solve_done: false,
                first_report_at_ms: None,
                tx: broadcast::channel(HUB_CHANNEL_CAP).0,
            });
        if !state.has_report {
            state.has_report = true;
        }
        if state.first_report_at_ms.is_none() {
            state.first_report_at_ms = first_report_at_ms.or_else(|| Some(now_ms()));
        }
    }

    /// Atomic (subscribe, snapshot-chunks): no overlap between replay and broadcast tail.
    #[must_use]
    pub fn subscribe_with_snapshot(
        &self,
        turn_id: &str,
    ) -> (broadcast::Receiver<HubMsg>, Vec<HubDeltaChunk>) {
        let mut guard = self.inner.lock().expect("live_report_hub lock");
        let state = guard
            .entry(turn_id.to_string())
            .or_insert_with(|| TurnStdoutState {
                text: String::new(),
                chunks: Vec::new(),
                has_report: false,
                solve_done: false,
                first_report_at_ms: None,
                tx: broadcast::channel(HUB_CHANNEL_CAP).0,
            });
        let rx = state.tx.subscribe();
        let snapshot = state.chunks.clone();
        (rx, snapshot)
    }

    /// Drop hub state when solve finished and no SSE subscribers remain.
    pub fn try_remove_turn(&self, turn_id: &str) {
        let mut guard = self.inner.lock().expect("live_report_hub lock");
        let Some(state) = guard.get(turn_id) else {
            return;
        };
        if !state.solve_done {
            return;
        }
        if state.tx.receiver_count() > 0 {
            return;
        }
        guard.remove(turn_id);
        tracing::debug!(
            target: "claw_live_report",
            turn_id = %turn_id,
            "live_report.hub_turn_removed"
        );
    }

    pub fn remove_turn(&self, turn_id: &str) {
        self.inner
            .lock()
            .expect("live_report_hub lock")
            .remove(turn_id);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{HubDeltaChunk, HubMsg, LiveReportHub};
    use serde_json::json;
    use tokio::sync::broadcast;
    use tokio::sync::broadcast::error::RecvError;

    fn delta(turn_id: &str, text: &str, hub: &LiveReportHub) {
        hub.ingest_json(turn_id, &json!({ "ev": "report.delta", "text": text }));
    }

    async fn recv_delta(rx: &mut broadcast::Receiver<HubMsg>) -> String {
        loop {
            match rx.recv().await {
                Ok(HubMsg::Delta(delta)) => return delta.text,
                Ok(HubMsg::SolveDone) => panic!("unexpected SolveDone"),
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => panic!("broadcast closed"),
            }
        }
    }

    #[tokio::test]
    async fn dual_subscribers_receive_same_live_deltas() {
        let hub = LiveReportHub::default();
        let turn_id = "T_dual_live";
        let (mut rx_a, snap_a) = hub.subscribe_with_snapshot(turn_id);
        assert!(snap_a.is_empty());

        delta(turn_id, "chunk-1", &hub);
        assert_eq!(recv_delta(&mut rx_a).await, "chunk-1");

        let (mut rx_b, snap_b) = hub.subscribe_with_snapshot(turn_id);
        assert_eq!(
            snap_b,
            vec![HubDeltaChunk {
                text: "chunk-1".to_string(),
                emit_seq: None,
            }]
        );

        delta(turn_id, "chunk-2", &hub);
        assert_eq!(recv_delta(&mut rx_a).await, "chunk-2");
        assert_eq!(recv_delta(&mut rx_b).await, "chunk-2");
        assert_eq!(hub.snapshot_text(turn_id), "chunk-1chunk-2");
    }

    #[tokio::test]
    async fn late_subscriber_replays_snapshot_then_live_tail() {
        let hub = LiveReportHub::default();
        let turn_id = "T_late_join";
        delta(turn_id, "a", &hub);
        delta(turn_id, "b", &hub);

        let (mut rx, snapshot) = hub.subscribe_with_snapshot(turn_id);
        assert_eq!(
            snapshot,
            vec![
                HubDeltaChunk {
                    text: "a".to_string(),
                    emit_seq: None,
                },
                HubDeltaChunk {
                    text: "b".to_string(),
                    emit_seq: None,
                },
            ]
        );
        delta(turn_id, "c", &hub);
        assert_eq!(recv_delta(&mut rx).await, "c");
    }

    #[tokio::test]
    async fn solve_done_removes_turn_when_no_subscribers() {
        let hub = LiveReportHub::default();
        let turn_id = "T_cleanup";
        delta(turn_id, "done-body", &hub);
        hub.ingest_json(turn_id, &json!({ "ev": "solve.done" }));
        assert!(!hub.has_report_for_turn(turn_id));
        assert!(!hub.is_solve_done(turn_id));
    }

    #[tokio::test]
    async fn subscriber_keeps_solve_done_flag_until_removed() {
        let hub = LiveReportHub::default();
        let turn_id = "T_done_flag";
        delta(turn_id, "body", &hub);
        let (_rx, snap) = hub.subscribe_with_snapshot(turn_id);
        assert_eq!(
            snap,
            vec![HubDeltaChunk {
                text: "body".to_string(),
                emit_seq: None,
            }]
        );
        hub.ingest_json(turn_id, &json!({ "ev": "solve.done" }));
        assert!(hub.is_solve_done(turn_id));
        assert!(hub.has_report_for_turn(turn_id));
    }

    #[test]
    fn ingest_stdout_line_parses_report_delta() {
        let hub = LiveReportHub::default();
        let turn_id = "T_stdout";
        let line = r#"__CLAW_GATEWAY_STDOUT__{"ev":"report.delta","text":"▸ 进度\n"}"#;
        hub.ingest_stdout_line(turn_id, line);
        assert_eq!(hub.snapshot_text(turn_id), "▸ 进度\n");
        assert!(hub.has_report_for_turn(turn_id));
    }

    #[test]
    fn promote_report_availability_latches_without_text() {
        let hub = LiveReportHub::default();
        let turn_id = "T_router";
        hub.promote_report_availability(turn_id, Some(1234));
        assert!(hub.has_report_for_turn(turn_id));
        assert_eq!(hub.snapshot_text(turn_id), "");
        assert_eq!(hub.first_report_at_ms_for_turn(turn_id), Some(1234));
    }

    #[test]
    fn promote_report_availability_keeps_earliest_timestamp() {
        let hub = LiveReportHub::default();
        let turn_id = "T_router_earliest";
        hub.promote_report_availability(turn_id, Some(1234));
        hub.promote_report_availability(turn_id, Some(5678));
        assert_eq!(hub.first_report_at_ms_for_turn(turn_id), Some(1234));
    }
}
