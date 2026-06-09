//! Pool daemon HTTP: live report SSE. Author: kejiqing

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use serde::Deserialize;
use tracing::info;

use super::docker_pool::DockerPoolManager;
use super::live_report_sse::live_report_sse_response;
use super::rpc::{dispatch_pool_rpc, PoolRpcReq, PoolRpcResp};

#[derive(Clone)]
pub struct PoolHttpState {
    pub pool: Arc<DockerPoolManager>,
}

#[derive(Debug, Deserialize)]
pub struct LiveReportQuery {
    #[serde(rename = "turnId")]
    pub turn_id: String,
    #[serde(rename = "taskId")]
    pub task_id: String,
    pub stream: Option<bool>,
    #[serde(rename = "requestId", default)]
    pub request_id: Option<String>,
    #[serde(rename = "projId", default)]
    pub proj_id: Option<i64>,
}

async fn get_biz_advice_report_live(
    State(st): State<PoolHttpState>,
    Query(q): Query<LiveReportQuery>,
) -> Result<Response, StatusCode> {
    if !q.stream.unwrap_or(false) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let turn_id = q.turn_id.trim();
    if turn_id.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let Some(hub) = st.pool.live_report_hub() else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    let task_id = q.task_id.trim().to_string();
    if task_id.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let request_id = q
        .request_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| task_id.clone());
    let proj_id = q.proj_id.unwrap_or(0);
    info!(
        target: "claw_live_report",
        component = "pool_http",
        phase = "live_sse_subscribe",
        turn_id = %turn_id,
        task_id = %task_id,
        proj_id,
        "pool /v1/biz_advice_report/live — hub subscribe (direct or via gateway proxy)"
    );
    Ok(live_report_sse_response(
        hub, turn_id, task_id, request_id, proj_id,
    ))
}

async fn healthz_live() -> impl IntoResponse {
    axum::Json(serde_json::json!({
        "ok": true,
        "contract": crate::live_report_audit::LIVE_REPORT_CONTRACT,
        "ingest": "pool-local",
        "rpc": "POST /v1/pool/rpc",
    }))
}

async fn post_pool_rpc(
    State(st): State<PoolHttpState>,
    Json(req): Json<PoolRpcReq>,
) -> Json<PoolRpcResp> {
    Json(dispatch_pool_rpc(&st.pool, req).await)
}

/// Serve pool HTTP (live SSE + JSON RPC) until error or graceful `shutdown` completes. Author: kejiqing
pub async fn serve_pool_http<F>(
    bind: &str,
    pool: Arc<DockerPoolManager>,
    shutdown: F,
) -> Result<(), String>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let state = PoolHttpState { pool };
    let app = Router::new()
        .route("/healthz/live-report", get(healthz_live))
        .route("/v1/pool/rpc", post(post_pool_rpc))
        .route(
            "/v1/biz_advice_report/live",
            get(get_biz_advice_report_live),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| format!("pool http bind {bind}: {e}"))?;
    info!(
        target: "claw_gateway_pool",
        component = "pool_http",
        phase = "listen",
        bind = %bind,
        "claw-pool-daemon http listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| format!("pool http serve: {e}"))
}
