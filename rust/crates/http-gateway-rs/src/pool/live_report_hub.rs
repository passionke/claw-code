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

/// Pending AskUserQuestion for Admin A2UI. Author: kejiqing
#[derive(Debug, Clone)]
pub struct AskUserPending {
    pub question_id: String,
    pub question: String,
    pub options: Option<Vec<String>>,
    pub a2ui: Value,
}

#[derive(Debug, Clone)]
pub enum HubMsg {
    Delta(HubDeltaChunk),
    AskUser(AskUserPending),
    AskUserCleared,
    SolveDone,
}

#[derive(Debug)]
struct TurnStdoutState {
    text: String,
    chunks: Vec<HubDeltaChunk>,
    has_report: bool,
    solve_done: bool,
    first_report_at_ms: Option<i64>,
    pending_ask: Option<AskUserPending>,
    tx: broadcast::Sender<HubMsg>,
}

fn empty_turn_state() -> TurnStdoutState {
    TurnStdoutState {
        text: String::new(),
        chunks: Vec::new(),
        has_report: false,
        solve_done: false,
        first_report_at_ms: None,
        pending_ask: None,
        tx: broadcast::channel(HUB_CHANNEL_CAP).0,
    }
}

fn parse_ask_pending(value: &Value) -> Option<AskUserPending> {
    let question_id = value
        .get("questionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let question = value
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let options = value.get("options").and_then(|o| {
        o.as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
    });
    let a2ui = value.get("a2ui").cloned().unwrap_or(Value::Null);
    Some(AskUserPending {
        question_id,
        question,
        options,
        a2ui,
    })
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
            .or_insert_with(empty_turn_state);
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
            "ask.user" => {
                let Some(pending) = parse_ask_pending(value) else {
                    tracing::warn!(
                        target: "claw_live_report",
                        turn_id = %turn_id,
                        "live_report.ingest_skipped — ask.user missing questionId"
                    );
                    return;
                };
                state.pending_ask = Some(pending.clone());
                let _ = state.tx.send(HubMsg::AskUser(pending));
            }
            "ask.user.cleared" => {
                state.pending_ask = None;
                let _ = state.tx.send(HubMsg::AskUserCleared);
            }
            "solve.done" => {
                state.solve_done = true;
                state.pending_ask = None;
                let _ = state.tx.send(HubMsg::SolveDone);
                drop(guard);
                self.try_remove_turn(turn_id);
            }
            "delegate.active" | "delegate.clear" => {
                // Handled by delegate_active_ingest before hub ingest.
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

    #[must_use]
    pub fn pending_ask_for_turn(&self, turn_id: &str) -> Option<AskUserPending> {
        self.inner
            .lock()
            .expect("live_report_hub lock")
            .get(turn_id)
            .and_then(|s| s.pending_ask.clone())
    }

    pub fn clear_pending_ask(&self, turn_id: &str) {
        let mut guard = self.inner.lock().expect("live_report_hub lock");
        if let Some(state) = guard.get_mut(turn_id) {
            state.pending_ask = None;
            let _ = state.tx.send(HubMsg::AskUserCleared);
        }
    }

    /// Latch report availability onto another turn without fabricating text deltas.
    /// Used by router turns to inherit active specialist report visibility. Author: kejiqing
    pub fn promote_report_availability(&self, turn_id: &str, first_report_at_ms: Option<i64>) {
        let mut guard = self.inner.lock().expect("live_report_hub lock");
        let state = guard
            .entry(turn_id.to_string())
            .or_insert_with(empty_turn_state);
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
            .or_insert_with(empty_turn_state);
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
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{HubDeltaChunk, HubMsg, LiveReportHub};
    use serde_json::json;

    fn ingest_delta(hub: &LiveReportHub, turn_id: &str, text: &str) {
        hub.ingest_json(turn_id, &json!({ "ev": "report.delta", "text": text }));
    }

    async fn recv_delta(rx: &mut tokio::sync::broadcast::Receiver<HubMsg>) -> String {
        loop {
            match rx.recv().await {
                Ok(HubMsg::Delta(delta)) => return delta.text,
                Ok(HubMsg::SolveDone) => panic!("unexpected SolveDone"),
                Ok(HubMsg::AskUser(_)) | Ok(HubMsg::AskUserCleared) => continue,
                Err(_) => continue,
            }
        }
    }

    #[tokio::test]
    async fn ask_user_pending_roundtrip() {
        let hub = LiveReportHub::default();
        let turn = "T_ask";
        hub.ingest_json(
            turn,
            &json!({
                "ev": "ask.user",
                "questionId": "aq_1",
                "question": "选哪个？",
                "options": ["A", "B"],
                "a2ui": {"catalogId": "claw-ask/v1"}
            }),
        );
        let pending = hub.pending_ask_for_turn(turn).expect("pending");
        assert_eq!(pending.question_id, "aq_1");
        assert_eq!(pending.question, "选哪个？");
        hub.clear_pending_ask(turn);
        assert!(hub.pending_ask_for_turn(turn).is_none());
    }

    #[tokio::test]
    async fn delta_and_snapshot() {
        let hub = LiveReportHub::default();
        let turn = "T1";
        let (mut rx, _) = hub.subscribe_with_snapshot(turn);
        ingest_delta(&hub, turn, "a");
        assert_eq!(recv_delta(&mut rx).await, "a");
        assert_eq!(hub.snapshot_text(turn), "a");
        let line = r#"__CLAW_GATEWAY_STDOUT__{"ev":"report.delta","text":"▸ 进度\n"}"#;
        hub.ingest_stdout_line(turn, line);
        assert!(hub.snapshot_text(turn).contains("进度"));
        let _ = HubDeltaChunk {
            text: String::new(),
            emit_seq: None,
        };
    }
}
