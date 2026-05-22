//! Proxy `GET /v1/biz_advice_report?stream=true` → worker `http://{container}:{port}/v1/turns/{turnId}/report`. Author: kejiqing

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use tracing::warn;

use crate::live_report_audit;
use crate::session_db::GatewaySessionDb;
use crate::ApiError;

/// In-container DNS name + fixed SSE port. Author: kejiqing
#[derive(Debug, Clone)]
pub struct WorkerStreamTarget {
    pub worker_host: String,
    pub port: u16,
}

impl WorkerStreamTarget {
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}:{}/v1/turns", self.worker_host, self.port)
    }

    pub fn report_url(&self, turn_id: &str) -> String {
        format!("{}/{turn_id}/report", self.base_url())
    }

    pub fn status_url(&self, turn_id: &str) -> String {
        format!("{}/{turn_id}/report/status", self.base_url())
    }
}

/// Active solve turn → worker endpoint (set on pool lease). Author: kejiqing
#[derive(Default)]
pub struct TurnWorkerStreamRegistry {
    by_turn: Mutex<HashMap<String, WorkerStreamTarget>>,
}

impl TurnWorkerStreamRegistry {
    pub async fn register(&self, turn_id: &str, target: WorkerStreamTarget) {
        self.by_turn
            .lock()
            .await
            .insert(turn_id.to_string(), target);
    }

    pub async fn remove(&self, turn_id: &str) {
        self.by_turn.lock().await.remove(turn_id);
    }

    pub async fn get(&self, turn_id: &str) -> Option<WorkerStreamTarget> {
        self.by_turn.lock().await.get(turn_id).cloned()
    }
}

/// Local registry first, then `gateway_turns.worker_report_*` for multi-gateway proxy. Author: kejiqing
pub async fn resolve_worker_stream_target(
    registry: &TurnWorkerStreamRegistry,
    db: &GatewaySessionDb,
    turn_id: &str,
    session_id: &str,
    ds_id: i64,
) -> Option<WorkerStreamTarget> {
    if let Some(target) = registry.get(turn_id).await {
        return Some(target);
    }
    let route = db
        .get_turn_worker_route(turn_id, session_id, ds_id)
        .await
        .unwrap_or_else(|e| {
            warn!(
                turn_id = %turn_id,
                error = %e,
                "get_turn_worker_route failed"
            );
            None
        });
    route.map(|(worker_host, port)| WorkerStreamTarget { worker_host, port })
}

#[derive(Deserialize)]
struct WorkerReportStatus {
    #[serde(rename = "hasReport", default)]
    has_report: bool,
    #[serde(rename = "reportTime", default)]
    report_time_ms: Option<i64>,
}

pub async fn worker_has_report(target: &WorkerStreamTarget, turn_id: &str) -> bool {
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok();
    let Some(client) = client else {
        return false;
    };
    let url = target.status_url(turn_id);
    let Ok(resp) = client.get(&url).send().await else {
        return false;
    };
    let Ok(st) = resp.json::<WorkerReportStatus>().await else {
        return false;
    };
    st.has_report
}

pub async fn worker_report_time_ms(target: &WorkerStreamTarget, turn_id: &str) -> Option<i64> {
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client
        .get(target.status_url(turn_id))
        .send()
        .await
        .ok()?;
    let st = resp.json::<WorkerReportStatus>().await.ok()?;
    if st.has_report {
        st.report_time_ms
    } else {
        None
    }
}

/// Transparent byte proxy from worker SSE to the admin client. Author: kejiqing
pub async fn proxy_worker_report_sse(
    target: WorkerStreamTarget,
    turn_id: &str,
) -> Result<Response, ApiError> {
    let url = target.report_url(turn_id);
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let resp = client
        .get(&url)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, format!("worker report GET: {e}")))?;
    if !resp.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("worker report HTTP {}", resp.status()),
        ));
    }
    let byte_stream = live_report_audit::trace_worker_proxy_byte_stream(
        turn_id,
        &url,
        resp.bytes_stream(),
    );
    Ok(Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
        .header(axum::http::header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(byte_stream))
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?)
}
