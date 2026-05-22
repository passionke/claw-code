//! Worker `text/event-stream` → in-memory fan-out; `GET /v1/biz_advice_report?stream=true` proxies bytes. Author: kejiqing

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, Stream, StreamExt};
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::{info, warn};

use crate::biz_advice_report::{sanitize_external_report_text, BizAdviceReportPayload};
use crate::biz_advice_report_live::{resolve_formal_report_text, LiveReportContext};
use crate::turn_live::verify_internal_token;
use crate::{ApiError, AppState};

const BROADCAST_CAP: usize = 512;
const REPLAY_CAP_BYTES: usize = 8 * 1024 * 1024;

/// Per-turn SSE relay (worker POST → admin GET). Author: kejiqing
#[derive(Default)]
pub struct TurnReportRelay {
    lanes: Mutex<HashMap<String, Arc<TurnReportLane>>>,
}

struct TurnReportLane {
    tx: broadcast::Sender<Bytes>,
    replay: RwLock<Vec<Bytes>>,
    replay_bytes: AtomicI64,
    has_bytes: AtomicBool,
    saw_done: AtomicBool,
    finished: AtomicBool,
    first_at_ms: AtomicI64,
}

impl TurnReportRelay {
    pub fn has_report(&self, turn_id: &str) -> bool {
        self.lanes
            .try_lock()
            .ok()
            .and_then(|m| m.get(turn_id).map(|l| l.has_bytes.load(Ordering::SeqCst)))
            .unwrap_or(false)
    }

    pub fn first_byte_at_ms(&self, turn_id: &str) -> Option<i64> {
        let lane = self.lanes.try_lock().ok()?.get(turn_id)?.clone();
        let t = lane.first_at_ms.load(Ordering::SeqCst);
        if t > 0 { Some(t) } else { None }
    }

    pub fn is_active(&self, turn_id: &str) -> bool {
        self.lanes
            .try_lock()
            .ok()
            .and_then(|m| {
                m.get(turn_id).map(|l| {
                    l.has_bytes.load(Ordering::SeqCst) && !l.finished.load(Ordering::SeqCst)
                })
            })
            .unwrap_or(false)
    }

    async fn lane(&self, turn_id: &str) -> Arc<TurnReportLane> {
        let mut map = self.lanes.lock().await;
        if let Some(l) = map.get(turn_id) {
            return Arc::clone(l);
        }
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        let lane = Arc::new(TurnReportLane {
            tx,
            replay: RwLock::new(Vec::new()),
            replay_bytes: AtomicI64::new(0),
            has_bytes: AtomicBool::new(false),
            saw_done: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            first_at_ms: AtomicI64::new(0),
        });
        map.insert(turn_id.to_string(), Arc::clone(&lane));
        lane
    }

    async fn publish(&self, turn_id: &str, chunk: Bytes) {
        if chunk.is_empty() {
            return;
        }
        let lane = self.lane(turn_id).await;
        if !lane.has_bytes.load(Ordering::SeqCst) {
            lane.has_bytes.store(true, Ordering::SeqCst);
            lane.first_at_ms.store(now_ms(), Ordering::SeqCst);
        }
        if chunk.windows(14).any(|w| w == b"biz.report.done") {
            lane.saw_done.store(true, Ordering::SeqCst);
        }
        {
            let mut replay = lane.replay.write().await;
            let add = i64::try_from(chunk.len()).unwrap_or(i64::MAX);
            while lane.replay_bytes.load(Ordering::SeqCst).saturating_add(add)
                > i64::try_from(REPLAY_CAP_BYTES).unwrap_or(i64::MAX)
                && !replay.is_empty()
            {
                let dropped = replay.remove(0);
                lane.replay_bytes.fetch_sub(
                    i64::try_from(dropped.len()).unwrap_or(0),
                    Ordering::SeqCst,
                );
            }
            lane.replay_bytes.fetch_add(add, Ordering::SeqCst);
            replay.push(chunk.clone());
        }
        let _ = lane.tx.send(chunk);
    }

    /// Byte stream for `GET …&stream=true` (replay then live). Author: kejiqing
    pub async fn subscribe_stream(
        self: Arc<Self>,
        turn_id: String,
    ) -> impl Stream<Item = Result<Bytes, Infallible>> + Send {
        let lane = self.lane(&turn_id).await;
        let replay = lane.replay.read().await.clone();
        let mut rx = lane.tx.subscribe();
        let replay_stream = stream::iter(replay.into_iter().map(Ok));
        let live = stream::unfold(rx, |mut rx| async move {
            match rx.recv().await {
                Ok(b) => Some((Ok(b), rx)),
                Err(broadcast::error::RecvError::Lagged(_)) => Some((Ok(Bytes::new()), rx)),
                Err(broadcast::error::RecvError::Closed) => None,
            }
        });
        replay_stream.chain(live)
    }

    async fn finish_lane(&self, turn_id: &str, ctx: &LiveReportContext, state: &AppState) {
        let Some(lane) = self.lanes.lock().await.get(turn_id).cloned() else {
            return;
        };
        lane.finished.store(true, Ordering::SeqCst);
        if lane.saw_done.load(Ordering::SeqCst) {
            return;
        }
        let status = crate::biz_advice_report_live::turn_status(
            &state.session_db,
            &ctx.turn_id,
            &ctx.session_id,
            ctx.ds_id,
        )
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "succeeded".to_string());
        let report = match resolve_formal_report_text(state, ctx).await {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    target: "claw_gateway_orchestration",
                    component = "turn_report_relay",
                    turn_id = %turn_id,
                    error = %e.detail(),
                    "report-stream end: formal report missing"
                );
                return;
            }
        };
        let payload = BizAdviceReportPayload {
            task_id: ctx.session_id.clone(),
            source_request_id: ctx.session_id.clone(),
            source_ds_id: ctx.ds_id,
            source_status: status,
            report_text: Some(sanitize_external_report_text(&report)),
            report_json: Some(serde_json::json!({
                "sessionId": ctx.session_id,
                "turnId": ctx.turn_id,
                "message": report,
            })),
        };
        let body = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
        let frame = format!("event: biz.report.done\ndata: {body}\n\n");
        self.publish(turn_id, Bytes::from(frame)).await;
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// `POST /v1/internal/turns/{turnId}/report-stream` — worker SSE upload, gateway fans out. Author: kejiqing
pub async fn post_report_stream(
    state: AppState,
    turn_id: String,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    verify_internal_token(&headers)?;
    if !crate::turn_id::validate_turn_id(&turn_id) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid turnId format"));
    }
    if !state
        .session_db
        .turn_exists(&turn_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("turn lookup: {e}")))?
    {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "unknown turnId"));
    }
    if state.live_ingest_closed.is_closed(&turn_id).await {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "report-stream closed for this turn",
        ));
    }

    info!(
        target: "claw_gateway_orchestration",
        component = "turn_report_relay",
        phase = "ingest_open",
        turn_id = %turn_id,
        "report-stream ingest opened"
    );

    let relay = Arc::clone(&state.report_relay);
    let ctx = resolve_relay_ctx(&state, &turn_id)
        .await
        .unwrap_or(LiveReportContext {
            session_id: String::new(),
            turn_id: turn_id.clone(),
            ds_id: 0,
            session_home: PathBuf::new(),
        });
    let state_for_finish = state.clone();

    let mut stream = body.into_data_stream();
    while let Some(frame) = stream.next().await {
        let chunk = frame.map_err(|e| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("read report-stream body: {e}"),
            )
        })?;
        relay.publish(&turn_id, chunk).await;
    }

    relay
        .finish_lane(&turn_id, &ctx, &state_for_finish)
        .await;
    state_for_finish
        .live_ingest_closed
        .mark_closed(&turn_id)
        .await;
    info!(
        target: "claw_gateway_orchestration",
        component = "turn_report_relay",
        phase = "ingest_done",
        turn_id = %turn_id,
        "report-stream ingest finished"
    );
    Ok(Response::new(Body::empty()))
}

async fn resolve_relay_ctx(state: &AppState, turn_id: &str) -> Option<LiveReportContext> {
    let (session_id, ds_id) = state.session_db.turn_session_scope(turn_id).await.ok()??;
    let session_home = state
        .session_db
        .get_session_home_rel(&session_id, ds_id)
        .await
        .ok()
        .flatten()
        .map(|rel| state.cfg.work_root.join(rel))
        .unwrap_or_else(PathBuf::new);
    Some(LiveReportContext {
        session_id,
        turn_id: turn_id.to_string(),
        ds_id,
        session_home,
    })
}

/// Proxy relay bytes to the browser (no PG tail worker). Author: kejiqing
pub async fn relay_proxy_response(
    relay: Arc<TurnReportRelay>,
    turn_id: String,
) -> Response {
    let no_buffer = axum::http::HeaderName::from_static("x-accel-buffering");
    let no_buffer_val = axum::http::HeaderValue::from_static("no");
    let body = Body::from_stream(relay.subscribe_stream(turn_id).await);
    (
        [(no_buffer, no_buffer_val)],
        (
            [
                (axum::http::header::CONTENT_TYPE, "text/event-stream; charset=utf-8"),
                (axum::http::header::CACHE_CONTROL, "no-cache"),
            ],
            body,
        ),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn relay_replays_then_live() {
        use futures_util::StreamExt;
        let relay = Arc::new(TurnReportRelay::default());
        relay
            .publish("T_10000000000000000000000000000001", Bytes::from("a"))
            .await;
        relay
            .publish("T_10000000000000000000000000000001", Bytes::from("b"))
            .await;
        let mut sub = Box::pin(
            relay
                .clone()
                .subscribe_stream("T_10000000000000000000000000000001".into())
                .await,
        );
        assert_eq!(sub.next().await.unwrap().unwrap(), Bytes::from("a"));
        assert_eq!(sub.next().await.unwrap().unwrap(), Bytes::from("b"));
        relay
            .publish("T_10000000000000000000000000000001", Bytes::from("c"))
            .await;
        assert_eq!(sub.next().await.unwrap().unwrap(), Bytes::from("c"));
    }

    #[tokio::test]
    async fn has_report_after_first_byte() {
        let relay = TurnReportRelay::default();
        assert!(!relay.has_report("T_10000000000000000000000000000001"));
        relay
            .publish("T_10000000000000000000000000000001", Bytes::from("x"))
            .await;
        assert!(relay.has_report("T_10000000000000000000000000000001"));
    }
}
