//! Worker-local SSE on fixed port (`CLAW_WORKER_REPORT_SSE_PORT`); gateway proxies GET. Author: kejiqing

use std::convert::Infallible;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

const COALESCE_MIN_CHARS: usize = 48;
const COALESCE_MAX_WAIT: Duration = Duration::from_millis(80);

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

fn delta_event(text: &str) -> Event {
    let data = serde_json::json!({ "text": text }).to_string();
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

async fn replay_pump(hub: Hub, tx: tokio_mpsc::UnboundedSender<Event>, subscriber_idx: u64) {
    let mut cursor = 0usize;
    loop {
        let (chunks, new_cursor) = hub.replay_tail(cursor);
        cursor = new_cursor;
        for (idx, chunk) in chunks.into_iter().enumerate() {
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
            if tx.send(delta_event(&chunk.text)).is_err() {
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

fn coalesce_loop(hub: Hub, rx: mpsc::Receiver<String>) {
    let mut buffer = String::new();
    let mut batch_trunk_first_ms: Option<i64> = None;
    let mut last_flush = Instant::now();
    while let Ok(piece) = rx.recv() {
        if batch_trunk_first_ms.is_none() {
            batch_trunk_first_ms = Some(report_sse_timing::now_ms());
        }
        buffer.push_str(&piece);
        while let Ok(more) = rx.try_recv() {
            buffer.push_str(&more);
        }
        let ready = buffer.chars().count() >= COALESCE_MIN_CHARS
            || last_flush.elapsed() >= COALESCE_MAX_WAIT;
        if ready && !buffer.is_empty() {
            let trunk_first = batch_trunk_first_ms.take().unwrap_or_else(report_sse_timing::now_ms);
            let hub_push_ms = report_sse_timing::now_ms();
            hub.push_delta(std::mem::take(&mut buffer), trunk_first, hub_push_ms);
            last_flush = Instant::now();
        }
    }
    if !buffer.is_empty() {
        let trunk_first = batch_trunk_first_ms.unwrap_or_else(report_sse_timing::now_ms);
        let hub_push_ms = report_sse_timing::now_ms();
        hub.push_delta(buffer, trunk_first, hub_push_ms);
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
