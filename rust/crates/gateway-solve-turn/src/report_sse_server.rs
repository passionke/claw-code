//! Worker-local SSE on fixed port (`CLAW_WORKER_REPORT_SSE_PORT`); gateway proxies GET. Author: kejiqing

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Json;
use futures_util::stream;
use futures_util::StreamExt;
use serde::Serialize;
use tokio::net::TcpListener;
use std::sync::mpsc;
use std::sync::Mutex;

use tokio::sync::{mpsc as tokio_mpsc, Notify};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::report_sse_timing;

/// Default in-container report SSE port (gateway proxies `http://{container}:{port}/…`). Author: kejiqing
pub const DEFAULT_REPORT_SSE_PORT: u16 = 18765;

#[derive(Clone)]
struct ReplayChunk {
    text: String,
    trunk_first_ms: i64,
    hub_push_ms: i64,
}

#[derive(Clone)]
struct Hub {
    turn_id: String,
    /// Coalesced deltas emitted so far; late SSE clients replay `[cursor..]` then tail on `notify`.
    replay: Arc<Mutex<Vec<ReplayChunk>>>,
    notify: Arc<Notify>,
    has_bytes: Arc<AtomicBool>,
    first_at_ms: Arc<AtomicI64>,
    /// Monotonic id per `GET …/report` subscriber (for timing logs). Author: kejiqing
    next_subscriber_id: Arc<AtomicI64>,
}

impl Hub {
    fn new(turn_id: &str) -> Self {
        Self {
            turn_id: turn_id.to_string(),
            replay: Arc::new(Mutex::new(Vec::new())),
            notify: Arc::new(Notify::new()),
            has_bytes: Arc::new(AtomicBool::new(false)),
            first_at_ms: Arc::new(AtomicI64::new(0)),
            next_subscriber_id: Arc::new(AtomicI64::new(1)),
        }
    }

    fn mark_has_bytes(&self, hub_push_ms: i64) {
        if !self.has_bytes.swap(true, Ordering::SeqCst) {
            self.first_at_ms.store(hub_push_ms, Ordering::SeqCst);
        }
    }

    fn push_delta(&self, text: String, trunk_first_ms: i64, hub_push_ms: i64) {
        if text.is_empty() {
            return;
        }
        let chars = text.chars().count();
        self.mark_has_bytes(hub_push_ms);
        let chunk_idx = {
            let mut replay = self.replay.lock().expect("replay lock");
            let idx = replay.len();
            report_sse_timing::log_hub_push(&self.turn_id, trunk_first_ms, hub_push_ms, chars, idx);
            replay.push(ReplayChunk {
                text,
                trunk_first_ms,
                hub_push_ms,
            });
            idx
        };
        let _ = chunk_idx; // logged inside lock
        self.notify.notify_waiters();
    }

    fn replay_tail(&self, cursor: usize) -> (Vec<ReplayChunk>, usize) {
        let replay = self.replay.lock().expect("replay lock");
        replay_tail(&replay, cursor)
    }

    fn alloc_subscriber_id(&self) -> u64 {
        self.next_subscriber_id
            .fetch_add(1, Ordering::SeqCst) as u64
    }
}

/// Slice of replay not yet sent on this SSE connection. Author: kejiqing
fn replay_tail(replay: &[ReplayChunk], cursor: usize) -> (Vec<ReplayChunk>, usize) {
    if cursor >= replay.len() {
        return (Vec::new(), cursor);
    }
    let tail = replay[cursor..].to_vec();
    (tail, replay.len())
}

fn delta_event(text: &str, hub_push_ms: i64) -> Event {
    let data = serde_json::json!({ "text": text, "t": hub_push_ms }).to_string();
    Event::default().event("biz.report.delta").data(data)
}

#[derive(Clone)]
struct App {
    hub: Hub,
}

#[derive(Serialize)]
struct ReportStatus {
    #[serde(rename = "hasReport")]
    has_report: bool,
    #[serde(rename = "reportTime", skip_serializing_if = "Option::is_none")]
    report_time_ms: Option<i64>,
}

async fn report_status(
    Path(turn_id): Path<String>,
    State(app): State<Arc<App>>,
) -> Json<ReportStatus> {
    if turn_id != app.hub.turn_id {
        return Json(ReportStatus {
            has_report: false,
            report_time_ms: None,
        });
    }
    let has = app.hub.has_bytes.load(Ordering::SeqCst);
    let t = app.hub.first_at_ms.load(Ordering::SeqCst);
    Json(ReportStatus {
        has_report: has,
        report_time_ms: if has && t > 0 { Some(t) } else { None },
    })
}

/// Catch-up replay spacing from original `hub_push_ms` gaps (16–80ms), not one browser tick burst.
fn replay_emit_delay_ms(prev_hub_push_ms: i64, hub_push_ms: i64) -> u64 {
    let gap = hub_push_ms.saturating_sub(prev_hub_push_ms);
    if gap == 0 {
        16
    } else {
        gap.clamp(16, 80) as u64
    }
}

async fn replay_pump(hub: Hub, tx: tokio_mpsc::UnboundedSender<Event>, subscriber_idx: u64) {
    let mut cursor = 0usize;
    let mut prev_hub_ms = 0i64;
    loop {
        let (chunks, new_cursor) = hub.replay_tail(cursor);
        cursor = new_cursor;
        for (idx, chunk) in chunks.into_iter().enumerate() {
            if prev_hub_ms > 0 {
                let wait = replay_emit_delay_ms(prev_hub_ms, chunk.hub_push_ms);
                tokio::time::sleep(Duration::from_millis(wait)).await;
            }
            prev_hub_ms = chunk.hub_push_ms;
            let sse_emit_ms = report_sse_timing::now_ms();
            let chars = chunk.text.chars().count();
            let chunk_idx = cursor + idx;
            report_sse_timing::log_sse_emit(
                &hub.turn_id,
                chunk.trunk_first_ms,
                chunk.hub_push_ms,
                sse_emit_ms,
                chars,
                chunk_idx,
                subscriber_idx,
            );
            if tx.send(delta_event(&chunk.text, chunk.hub_push_ms)).is_err() {
                return;
            }
        }
        hub.notify.notified().await;
    }
}

async fn report_sse(
    Path(turn_id): Path<String>,
    State(app): State<Arc<App>>,
) -> impl IntoResponse {
    if turn_id != app.hub.turn_id {
        return (axum::http::StatusCode::NOT_FOUND, "turn mismatch").into_response();
    }
    let task_id = app.hub.turn_id.clone();
    let hub = app.hub.clone();
    let start_data = serde_json::json!({ "taskId": task_id }).to_string();
    let start_ev = Event::default()
        .event("biz.report.start")
        .data(start_data);

    let subscriber_idx = app.hub.alloc_subscriber_id();
    report_sse_timing::log_sse_subscriber_open(&app.hub.turn_id, subscriber_idx);
    let (delta_tx, delta_rx) = tokio_mpsc::unbounded_channel();
    tokio::spawn(replay_pump(hub, delta_tx, subscriber_idx));

    let deltas = stream::unfold(delta_rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|ev| (Ok::<Event, Infallible>(ev), rx))
    });
    let body = stream::once(async move { Ok(start_ev) }).chain(deltas);
    Sse::new(body)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// Push coalesced model text into the worker SSE hub. Author: kejiqing
#[derive(Clone)]
pub struct ReportStreamHandle {
    hub: Hub,
    coalesce_tx: mpsc::Sender<String>,
}

impl ReportStreamHandle {
    pub fn push_text_delta(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        report_sse_timing::log_trunk_in(&self.hub.turn_id, text.chars().count());
        let _ = self.coalesce_tx.send(text.to_string());
    }

    pub fn has_report(&self) -> bool {
        self.hub.has_bytes.load(Ordering::SeqCst)
    }
}

/// Keeps the report SSE listener alive for the solve process. Author: kejiqing
pub struct ReportSseServerGuard {
    join: JoinHandle<()>,
    coalesce_join: JoinHandle<()>,
}

impl Drop for ReportSseServerGuard {
    fn drop(&mut self) {
        self.coalesce_join.abort();
        self.join.abort();
    }
}

/// One model `push_text_delta` → one hub chunk (no 48-char / 80ms / try_recv batching). Author: kejiqing
fn coalesce_loop(hub: Hub, rx: mpsc::Receiver<String>) {
    while let Ok(piece) = rx.recv() {
        let trunk_first = report_sse_timing::now_ms();
        let hub_push_ms = report_sse_timing::now_ms();
        hub.push_delta(piece, trunk_first, hub_push_ms);
    }
}

/// Bind `0.0.0.0:port` and serve `GET /v1/turns/{turnId}/report` (+ `/status`). Author: kejiqing
pub fn spawn(
    turn_id: &str,
    port: u16,
) -> Result<(ReportStreamHandle, ReportSseServerGuard), String> {
    let hub = Hub::new(turn_id);
    let (coalesce_tx, coalesce_rx) = mpsc::channel();
    let coalesce_hub = hub.clone();
    let coalesce_join = tokio::task::spawn_blocking(move || coalesce_loop(coalesce_hub, coalesce_rx));

    let app = Arc::new(App { hub: hub.clone() });
    let router = axum::Router::new()
        .route(
            "/v1/turns/{turn_id}/report",
            get(report_sse),
        )
        .route(
            "/v1/turns/{turn_id}/report/status",
            get(report_status),
        )
        .with_state(app);

    let log_turn = turn_id.to_string();
    let join = tokio::spawn(async move {
        let addr = format!("0.0.0.0:{port}");
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                warn!(
                    target: "claw_gateway_solve",
                    component = "report_sse_server",
                    port,
                    error = %e,
                    "report SSE bind failed"
                );
                return;
            }
        };
        info!(
            target: "claw_gateway_solve",
            component = "report_sse_server",
            port,
            turn_id = %log_turn,
            "worker report SSE listening"
        );
        if let Err(e) = axum::serve(listener, router).await {
            warn!(
                target: "claw_gateway_solve",
                component = "report_sse_server",
                error = %e,
                "report SSE server exited"
            );
        }
    });

    let handle = ReportStreamHandle { hub, coalesce_tx };
    Ok((
        handle,
        ReportSseServerGuard {
            join,
            coalesce_join,
        },
    ))
}

#[must_use]
pub fn resolve_report_sse_port() -> Option<u16> {
    let raw = std::env::var("CLAW_WORKER_REPORT_SSE_PORT").ok()?;
    let port: u16 = raw.trim().parse().ok()?;
    if port == 0 { None } else { Some(port) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port_is_fixed() {
        assert_eq!(DEFAULT_REPORT_SSE_PORT, 18765);
    }

    #[test]
    fn replay_tail_returns_unsent_chunks_and_advances_cursor() {
        let replay = vec![
            ReplayChunk {
                text: "a".into(),
                trunk_first_ms: 1,
                hub_push_ms: 2,
            },
            ReplayChunk {
                text: "b".into(),
                trunk_first_ms: 3,
                hub_push_ms: 4,
            },
        ];
        let (tail, cur) = replay_tail(&replay, 0);
        assert_eq!(tail.len(), 2);
        assert_eq!(cur, 2);
        let (tail, cur) = replay_tail(&replay, 2);
        assert!(tail.is_empty());
        assert_eq!(cur, 2);
    }

    #[test]
    fn replay_emit_delay_ms_uses_hub_gap_not_zero() {
        assert_eq!(replay_emit_delay_ms(100, 100), 16);
        assert_eq!(replay_emit_delay_ms(100, 200), 80);
        assert_eq!(replay_emit_delay_ms(100, 130), 30);
    }

    /// Late SSE client must not receive all catch-up deltas in one instant (paced by hub_push_ms).
    #[tokio::test]
    async fn late_subscriber_catch_up_is_time_spaced() {
        use std::time::Instant;

        use futures_util::StreamExt;

        let turn_id = "T_pace_late_subscriber00000001";
        let port = 38767_u16;
        let (handle, _guard) = spawn(turn_id, port).expect("spawn");
        tokio::time::sleep(Duration::from_millis(60)).await;

        let h = handle.clone();
        tokio::task::spawn_blocking(move || {
            for i in 0..4 {
                h.push_text_delta(&format!("seg{i}"));
                if i < 3 {
                    std::thread::sleep(Duration::from_millis(60));
                }
            }
        });
        tokio::time::sleep(Duration::from_millis(320)).await;

        let url = format!("http://127.0.0.1:{port}/v1/turns/{turn_id}/report");
        let resp = reqwest::get(&url).await.expect("GET SSE");
        assert!(resp.status().is_success());
        let mut stream = resp.bytes_stream();
        let mut buf = Vec::new();
        let mut delta_at: Vec<Instant> = Vec::new();
        let marker = b"event: biz.report.delta";
        while let Some(Ok(bytes)) = stream.next().await {
            buf.extend_from_slice(&bytes);
            while let Some(pos) = buf.windows(marker.len()).position(|w| w == marker) {
                delta_at.push(Instant::now());
                buf.drain(..pos + marker.len());
            }
            if delta_at.len() >= 4 {
                break;
            }
        }
        assert!(delta_at.len() >= 3, "expected multiple deltas, got {}", delta_at.len());
        let mut gaps_ms: Vec<u128> = delta_at
            .windows(2)
            .map(|w| w[1].duration_since(w[0]).as_millis())
            .collect();
        gaps_ms.sort();
        let max_gap = *gaps_ms.last().unwrap_or(&0);
        assert!(
            max_gap >= 12,
            "catch-up should be paced (max inter-delta gap {max_gap}ms), gaps={gaps_ms:?}"
        );
    }

    #[test]
    fn hub_replay_catches_late_subscriber() {
        let hub = Hub::new("T_test");
        hub.push_delta("one".into(), 10, 11);
        hub.push_delta("two".into(), 12, 13);
        let (first, c1) = hub.replay_tail(0);
        assert_eq!(first.len(), 2);
        assert_eq!(c1, 2);
        hub.push_delta("three".into(), 14, 15);
        let (second, c2) = hub.replay_tail(c1);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].text, "three");
        assert_eq!(c2, 3);
        let (again, c3) = hub.replay_tail(0);
        assert_eq!(again.len(), 3);
        assert_eq!(c3, 3);
    }

    /// Model trunk → hub → SSE subscriber; logs `claw_report_sse_timing` when env set. Author: kejiqing
    #[tokio::test]
    async fn e2e_report_sse_timing_phases() {
        use std::time::Duration;

        use futures_util::StreamExt;

        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::new("claw_report_sse_timing=trace,info"),
            )
            .with_test_writer()
            .try_init();

        crate::report_sse_timing::force_enabled_for_test();

        let turn_id = "T_e2e_timing00000000000000000001";
        let port = 38765_u16;
        let (handle, _guard) = spawn(turn_id, port).expect("spawn report SSE");
        tokio::time::sleep(Duration::from_millis(80)).await;

        let turn_client = turn_id.to_string();
        let reader = tokio::spawn(async move {
            let url = format!("http://127.0.0.1:{port}/v1/turns/{turn_client}/report");
            let resp = reqwest::get(&url).await.expect("GET report SSE");
            assert!(resp.status().is_success());
            let mut stream = resp.bytes_stream();
            let mut chunks = 0usize;
            while let Some(item) = stream.next().await {
                if item.is_ok() {
                    chunks += 1;
                    if chunks >= 2 {
                        break;
                    }
                }
            }
            chunks
        });

        tokio::time::sleep(Duration::from_millis(30)).await;
        handle.push_text_delta(&"delta-".repeat(32));
        tokio::time::sleep(Duration::from_millis(200)).await;

        let chunks = reader.await.expect("reader join");
        assert!(
            chunks >= 1,
            "SSE reader should receive start + at least one frame"
        );
        assert!(
            handle.has_report(),
            "hub should have bytes after coalesced push"
        );
    }
}
