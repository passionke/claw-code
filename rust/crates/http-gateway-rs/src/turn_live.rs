//! Live assistant-stream chunks in PostgreSQL + `LISTEN/NOTIFY` wake for report SSE. Author: kejiqing

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use sqlx::postgres::PgListener;
use tokio::sync::{Mutex, Notify};
use tracing::warn;

use crate::live_report_ports::{AssistantStreamStore, SessionDbIngestAdapter};
use crate::{ApiError, AppState};

/// Dependencies for assistant-stream ingest (testable without full [`AppState`]). Author: kejiqing
#[derive(Clone)]
pub struct AssistantStreamIngestCtx {
    pub store: Arc<dyn AssistantStreamStore>,
    pub live_ingest_closed: LiveIngestRegistry,
}

impl From<AppState> for AssistantStreamIngestCtx {
    fn from(state: AppState) -> Self {
        Self {
            store: Arc::new(SessionDbIngestAdapter(state.session_db)),
            live_ingest_closed: state.live_ingest_closed,
        }
    }
}

/// Turns whose assistant-stream body has ended (409 on new ingest). Author: kejiqing
#[derive(Clone, Default)]
pub struct LiveIngestRegistry {
    closed: Arc<Mutex<HashMap<String, ()>>>,
}

impl LiveIngestRegistry {
    pub async fn mark_closed(&self, turn_id: &str) {
        self.closed
            .lock()
            .await
            .insert(turn_id.to_string(), ());
    }

    pub async fn is_closed(&self, turn_id: &str) -> bool {
        self.closed.lock().await.contains_key(turn_id)
    }
}

/// Per-process `LISTEN claw_turn_live` → `Notify` per `turn_id`. Author: kejiqing
pub struct LiveNotifyHub {
    waiters: Arc<Mutex<HashMap<String, Arc<Notify>>>>,
}

impl LiveNotifyHub {
    pub fn new() -> Self {
        Self {
            waiters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn subscribe(&self, turn_id: &str) -> Arc<Notify> {
        let mut map = self.waiters.lock().await;
        map.entry(turn_id.to_string())
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    async fn wake(&self, turn_id: &str) {
        let notify = {
            let map = self.waiters.lock().await;
            map.get(turn_id).cloned()
        };
        if let Some(n) = notify {
            n.notify_waiters();
        }
    }

    /// Test-only: simulate `LISTEN/NOTIFY` wake without PostgreSQL. Author: kejiqing
    #[cfg(test)]
    pub async fn signal_turn(&self, turn_id: &str) {
        self.wake(turn_id).await;
    }

    pub fn spawn_listener(database_url: String, hub: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                match PgListener::connect(&database_url).await {
                    Ok(mut listener) => {
                        if let Err(e) =
                            listener.listen(crate::session_db::LIVE_CHUNK_NOTIFY_CHANNEL).await
                        {
                            warn!(
                                target: "claw_gateway_orchestration",
                                component = "turn_live",
                                error = %e,
                                "LISTEN claw_turn_live failed"
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            continue;
                        }
                        loop {
                            match listener.recv().await {
                                Ok(notif) => {
                                    let payload = notif.payload().to_string();
                                    if let Some(tid) = parse_notify_turn_id(&payload) {
                                        hub.wake(&tid).await;
                                    }
                                }
                                Err(e) => {
                                    warn!(
                                        target: "claw_gateway_orchestration",
                                        component = "turn_live",
                                        error = %e,
                                        "LISTEN recv ended; reconnecting"
                                    );
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            target: "claw_gateway_orchestration",
                            component = "turn_live",
                            error = %e,
                            "PgListener connect failed; retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        });
    }
}

fn parse_notify_turn_id(payload: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(payload).ok()?;
    v.get("turnId")
        .and_then(|x| x.as_str())
        .map(str::to_string)
}

#[derive(Debug, Deserialize)]
struct StreamChunkLine {
    chunk: String,
}

fn internal_token_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-claw-gateway-internal-token")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "))
                .map(str::to_string)
        })
}

fn verify_internal_token(headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = std::env::var("CLAW_GATEWAY_INTERNAL_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
    else {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "CLAW_GATEWAY_INTERNAL_TOKEN not configured",
        ));
    };
    let got = internal_token_from_headers(headers).unwrap_or_default();
    if got != expected {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid internal token",
        ));
    }
    Ok(())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// `POST /v1/internal/turns/{turnId}/assistant-stream` — NDJSON `{"chunk":"..."}` per line. Author: kejiqing
pub async fn post_assistant_stream(
    state: AppState,
    turn_id: String,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    post_assistant_stream_with_ctx(state.into(), turn_id, headers, body).await
}

pub async fn post_assistant_stream_with_ctx(
    ctx: AssistantStreamIngestCtx,
    turn_id: String,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    verify_internal_token(&headers)?;
    if !crate::turn_id::validate_turn_id(&turn_id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid turnId format",
        ));
    }
    if !ctx
        .store
        .has_turn(&turn_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("turn lookup: {e}")))?
    {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "unknown turnId"));
    }
    if ctx.live_ingest_closed.is_closed(&turn_id).await {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "assistant-stream ingest closed for this turn",
        ));
    }

    let mut stream = body.into_data_stream();
    let mut buf = Vec::new();
    let mut batch: Vec<String> = Vec::new();
    let mut last_flush = tokio::time::Instant::now();
    const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(75);
    const BATCH_MAX_CHARS: usize = 4096;

    while let Some(frame) = stream.next().await {
        let chunk = frame.map_err(|e| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("read assistant-stream body: {e}"),
            )
        })?;
        buf.extend_from_slice(&chunk);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let line_bytes = buf.drain(..=pos).collect::<Vec<_>>();
            let line = std::str::from_utf8(&line_bytes[..line_bytes.len().saturating_sub(1)])
                .map_err(|_| {
                    ApiError::new(StatusCode::BAD_REQUEST, "assistant-stream line not valid UTF-8")
                })?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let row: StreamChunkLine = serde_json::from_str(line).map_err(|e| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("invalid NDJSON line: {e}"),
                )
            })?;
            if !row.chunk.is_empty() {
                batch.push(row.chunk);
            }
            let batch_chars: usize = batch.iter().map(|s| s.chars().count()).sum();
            if batch_chars >= BATCH_MAX_CHARS || last_flush.elapsed() >= FLUSH_INTERVAL {
                flush_live_batch(&ctx.store, &turn_id, &mut batch).await?;
                last_flush = tokio::time::Instant::now();
            }
        }
    }
    if !buf.is_empty() {
        let line = std::str::from_utf8(&buf).map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "assistant-stream trailing bytes not valid UTF-8",
            )
        })?;
        let line = line.trim();
        if !line.is_empty() {
            let row: StreamChunkLine = serde_json::from_str(line).map_err(|e| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("invalid NDJSON tail: {e}"),
                )
            })?;
            if !row.chunk.is_empty() {
                batch.push(row.chunk);
            }
        }
    }
    flush_live_batch(&ctx.store, &turn_id, &mut batch).await?;
    ctx.live_ingest_closed.mark_closed(&turn_id).await;
    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(json!({"ok": true}).to_string()))
        .unwrap())
}

async fn flush_live_batch(
    store: &Arc<dyn AssistantStreamStore>,
    turn_id: &str,
    batch: &mut Vec<String>,
) -> Result<(), ApiError> {
    if batch.is_empty() {
        return Ok(());
    }
    let chunks: Vec<String> = batch.drain(..).collect();
    store
        .append_live_chunks(turn_id, &chunks, now_ms())
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("append live chunks: {e}"),
            )
        })?;
    Ok(())
}

/// Parse `pg_notify` payload (`{"turnId":"T_…"}`). Author: kejiqing
#[must_use]
pub(crate) fn parse_live_notify_turn_id(payload: &str) -> Option<String> {
    parse_notify_turn_id(payload)
}

#[cfg(test)]
mod ingest_tests {
    use super::*;
    use crate::live_report_mocks::MockIngestStore;
    use axum::body::Body;
    use axum::http::{HeaderMap, HeaderValue, StatusCode};
    use std::sync::{Arc, Mutex};

    static TOKEN_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct TokenEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        prev: Option<String>,
    }

    impl TokenEnvGuard {
        fn set(token: &str) -> Self {
            let lock = TOKEN_ENV_LOCK.lock().expect("token env lock");
            let prev = std::env::var("CLAW_GATEWAY_INTERNAL_TOKEN").ok();
            std::env::set_var("CLAW_GATEWAY_INTERNAL_TOKEN", token);
            Self {
                _lock: lock,
                prev,
            }
        }
    }

    impl Drop for TokenEnvGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.prev {
                std::env::set_var("CLAW_GATEWAY_INTERNAL_TOKEN", v);
            } else {
                std::env::remove_var("CLAW_GATEWAY_INTERNAL_TOKEN");
            }
        }
    }

    fn auth_headers(token: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            "x-claw-gateway-internal-token",
            HeaderValue::from_str(token).unwrap(),
        );
        h
    }

    async fn mock_ctx(turn_id: &str) -> (AssistantStreamIngestCtx, Arc<MockIngestStore>) {
        let mock = Arc::new(MockIngestStore::default());
        mock.register_turn(turn_id).await;
        let store: Arc<dyn AssistantStreamStore> = mock.clone();
        let ctx = AssistantStreamIngestCtx {
            store,
            live_ingest_closed: LiveIngestRegistry::default(),
        };
        (ctx, mock)
    }

    #[test]
    fn parse_live_notify_turn_id_extracts_turn_id() {
        assert_eq!(
            parse_live_notify_turn_id(r#"{"turnId":"T_abc","maxSeq":2,"kind":"chunk"}"#).as_deref(),
            Some("T_abc")
        );
        assert_eq!(
            parse_live_notify_turn_id(r#"{"turnId":"T_x","kind":"terminal"}"#).as_deref(),
            Some("T_x")
        );
        assert!(parse_live_notify_turn_id("not-json").is_none());
    }

    #[tokio::test]
    async fn live_notify_hub_signal_turn_wakes_subscriber_without_postgres() {
        let hub = Arc::new(LiveNotifyHub::new());
        let turn_id = "T_10000000000000000000000000000002";
        let notify = hub.subscribe(turn_id).await;
        let waiter = tokio::spawn(async move {
            notify.notified().await;
        });
        tokio::task::yield_now().await;
        hub.signal_turn(turn_id).await;
        tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("signal_turn should wake subscriber")
            .expect("waiter join");
    }

    #[tokio::test]
    async fn post_assistant_stream_rejects_bad_token_and_invalid_utf8() {
        let turn_id = "T_10000000000000000000000000000003";
        let (ctx, _mock) = mock_ctx(turn_id).await;
        let _guard = TokenEnvGuard::set("ingest-secret");

        let err = post_assistant_stream_with_ctx(
            ctx.clone(),
            turn_id.to_string(),
            HeaderMap::new(),
            Body::from(r#"{"chunk":"x"}"#),
        )
        .await
        .expect_err("missing token");
        assert_eq!(err.status, StatusCode::UNAUTHORIZED);

        let bad_utf8 = Body::from(vec![0xff, b'\n']);
        let err = post_assistant_stream_with_ctx(
            ctx,
            turn_id.to_string(),
            auth_headers("ingest-secret"),
            bad_utf8,
        )
        .await
        .expect_err("invalid utf8");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.detail().contains("UTF-8"));
    }

    #[tokio::test]
    async fn post_assistant_stream_writes_chunks_to_mock_store_and_409_on_replay() {
        let turn_id = "T_10000000000000000000000000000004";
        let (ctx, mock) = mock_ctx(turn_id).await;
        let _guard = TokenEnvGuard::set("ingest-secret");
        let body = Body::from(
            "{\"chunk\":\"hello\"}\n{\"chunk\":\"世界\"}\n".as_bytes().to_vec(),
        );
        let resp = post_assistant_stream_with_ctx(
            ctx.clone(),
            turn_id.to_string(),
            auth_headers("ingest-secret"),
            body,
        )
        .await
        .expect("ingest ok");
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(ctx.live_ingest_closed.is_closed(turn_id).await);
        let stored = mock.chunks_for(turn_id).await;
        assert_eq!(stored, vec!["hello", "世界"]);

        let err = post_assistant_stream_with_ctx(
            ctx,
            turn_id.to_string(),
            auth_headers("ingest-secret"),
            Body::from("{\"chunk\":\"again\"}\n"),
        )
        .await
        .expect_err("second ingest");
        assert_eq!(err.status, StatusCode::CONFLICT);
    }
}
