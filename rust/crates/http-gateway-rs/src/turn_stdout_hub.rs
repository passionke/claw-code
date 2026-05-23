//! In-memory hub for worker stdout report deltas (ingest + live SSE). Author: kejiqing

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::sync::broadcast;

const HUB_CHANNEL_CAP: usize = 256;

/// Broadcast message: `Delta` for streaming chunks, `SolveDone` as an in-band
/// terminal sentinel. Sending `SolveDone` through the same channel guarantees
/// FIFO ordering with prior deltas — receivers won't miss tail chunks that
/// arrived right before solve.done (which used to happen when the receiver
/// broke out on a `solve_done` status flag). Author: kejiqing
#[derive(Debug, Clone)]
pub enum HubMsg {
    Delta(String),
    SolveDone,
}

#[derive(Debug)]
struct TurnStdoutState {
    text: String,
    has_report: bool,
    solve_done: bool,
    first_report_at_ms: Option<i64>,
    tx: broadcast::Sender<HubMsg>,
}

#[derive(Clone, Default)]
pub struct TurnStdoutHub {
    inner: std::sync::Arc<Mutex<HashMap<String, TurnStdoutState>>>,
}

impl std::fmt::Debug for TurnStdoutHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnStdoutHub").finish_non_exhaustive()
    }
}

impl TurnStdoutHub {
    pub fn ingest_json(&self, turn_id: &str, value: &Value) {
        let ev = value.get("ev").and_then(Value::as_str).unwrap_or("");
        let mut guard = self.inner.lock().expect("turn_stdout_hub lock");
        let state = guard.entry(turn_id.to_string()).or_insert_with(|| TurnStdoutState {
            text: String::new(),
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
                let chunk_len = chunk.len();
                state.text.push_str(chunk);
                let _ = state.tx.send(HubMsg::Delta(chunk.to_string()));
                crate::biz_report_sse_log::log_stdout_ingest(turn_id, chunk_len);
            }
            "solve.done" => {
                state.solve_done = true;
                // In-band terminal sentinel: FIFO with previously-sent deltas, so any
                // subscriber draining the channel sees every delta before SolveDone.
                let _ = state.tx.send(HubMsg::SolveDone);
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

    #[must_use]
    pub fn has_report(&self, turn_id: &str) -> bool {
        self.inner
            .lock()
            .expect("turn_stdout_hub lock")
            .get(turn_id)
            .is_some_and(|s| s.has_report)
    }

    #[must_use]
    pub fn solve_done(&self, turn_id: &str) -> bool {
        self.inner
            .lock()
            .expect("turn_stdout_hub lock")
            .get(turn_id)
            .is_some_and(|s| s.solve_done)
    }

    #[must_use]
    pub fn snapshot_text(&self, turn_id: &str) -> String {
        self.inner
            .lock()
            .expect("turn_stdout_hub lock")
            .get(turn_id)
            .map(|s| s.text.clone())
            .unwrap_or_default()
    }

    pub fn first_report_at_ms(&self, turn_id: &str) -> Option<i64> {
        self.inner
            .lock()
            .expect("turn_stdout_hub lock")
            .get(turn_id)
            .and_then(|s| s.first_report_at_ms)
    }

    pub fn subscribe(&self, turn_id: &str) -> broadcast::Receiver<HubMsg> {
        let mut guard = self.inner.lock().expect("turn_stdout_hub lock");
        guard
            .entry(turn_id.to_string())
            .or_insert_with(|| TurnStdoutState {
                text: String::new(),
                has_report: false,
                solve_done: false,
                first_report_at_ms: None,
                tx: broadcast::channel(HUB_CHANNEL_CAP).0,
            })
            .tx
            .subscribe()
    }

    /// Atomic (subscribe, snapshot): broadcast receiver registers BEFORE the snapshot
    /// is cloned, both under one lock. The receiver will only deliver `tx.send` calls
    /// that happen-after this method returns; the snapshot covers every chunk
    /// already merged into `state.text` at that instant. No overlap, no gap. Author: kejiqing
    pub fn subscribe_with_snapshot(
        &self,
        turn_id: &str,
    ) -> (broadcast::Receiver<HubMsg>, String) {
        let mut guard = self.inner.lock().expect("turn_stdout_hub lock");
        let state = guard
            .entry(turn_id.to_string())
            .or_insert_with(|| TurnStdoutState {
                text: String::new(),
                has_report: false,
                solve_done: false,
                first_report_at_ms: None,
                tx: broadcast::channel(HUB_CHANNEL_CAP).0,
            });
        let rx = state.tx.subscribe();
        let snapshot = state.text.clone();
        (rx, snapshot)
    }

    /// Reserved for future hub entry cleanup after turn TTL (not wired yet). Author: kejiqing
    pub fn remove_turn(&self, turn_id: &str) {
        self.inner
            .lock()
            .expect("turn_stdout_hub lock")
            .remove(turn_id);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Forward one claw stdout line to the gateway ingest API (pool daemon / host exec).
pub async fn forward_claw_stdout_line(turn_id: &str, line: &str) {
    let Some(value) = gateway_solve_turn::gateway_stdout::parse_stdout_line(line) else {
        crate::live_report_audit::debug_non_claw_stdout_line(turn_id, line);
        return;
    };
    let env = crate::live_report_audit::LiveReportForwardEnv::read();
    if !env.ready() {
        crate::live_report_audit::warn_forward_env_missing_once(turn_id);
        return;
    }
    let base = std::env::var("CLAW_GATEWAY_INTERNAL_BASE_URL")
        .expect("checked ready");
    let token = std::env::var("CLAW_GATEWAY_INTERNAL_TOKEN")
        .expect("checked ready");
    let url = format!(
        "{}/v1/internal/turns/{}/stdout-event",
        base.trim().trim_end_matches('/'),
        turn_id
    );
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            crate::live_report_audit::warn_forward_client_build_failed_once(turn_id, &e.to_string());
            return;
        }
    };
    match client
        .post(url)
        .header("Authorization", format!("Bearer {}", token.trim()))
        .json(&value)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            tracing::info!(
                target: "claw_live_report",
                turn_id = %turn_id,
                status = %resp.status(),
                ev = "report.delta",
                "live_report.forward_ok"
            );
        }
        Ok(resp) => {
            tracing::error!(
                target: "claw_live_report",
                turn_id = %turn_id,
                status = %resp.status(),
                base_url = %base.trim(),
                "live_report.forward_http_error"
            );
        }
        Err(e) => {
            tracing::error!(
                target: "claw_live_report",
                turn_id = %turn_id,
                base_url = %base.trim(),
                error = %e,
                "live_report.forward_network_error"
            );
        }
    }
}
