//! Sandbox HTTP: health + RPC. Author: kejiqing

use std::sync::Arc;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use tracing::info;

use super::docker_pool::DockerPoolManager;
use super::rpc::dispatch_pool_rpc;
use super::sandbox_rpc::dispatch_sandbox_rpc;
use super::sandbox_stream::{sandbox_rpc_ndjson_stream, sandbox_rpc_wants_stream};
use claw_sandbox_protocol::{PoolRpcReq, PoolRpcResp, SandboxRpcReq, SandboxRpcResp};

#[derive(Clone)]
pub struct PoolHttpState {
    pub pool: Arc<DockerPoolManager>,
}

async fn healthz() -> impl IntoResponse {
    Json(serde_json::json!({
        "ok": true,
        "rpc": "POST /v1/sandbox/rpc",
        "legacy_rpc": "POST /v1/pool/rpc",
        "exec_stream": "ExecSolve/Exec default stream=true → application/x-ndjson",
    }))
}

async fn post_sandbox_rpc(
    State(st): State<PoolHttpState>,
    Json(req): Json<SandboxRpcReq>,
) -> Response {
    if sandbox_rpc_wants_stream(&req) {
        let stream = sandbox_rpc_ndjson_stream(Arc::clone(&st.pool), req);
        let body = Body::from_stream(stream);
        return Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/x-ndjson"),
            )
            .body(body)
            .unwrap_or_else(|_| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(err_json("stream response build failed")),
                )
                    .into_response()
            });
    }
    Json(dispatch_sandbox_rpc(&st.pool, req).await).into_response()
}

fn err_json(msg: &str) -> SandboxRpcResp {
    SandboxRpcResp {
        ok: false,
        error: Some(msg.to_string()),
        lease: None,
        outcome: None,
        files: None,
        capacity: None,
        exec_chunk: None,
        leased_slots: None,
    }
}

async fn post_pool_rpc(
    State(st): State<PoolHttpState>,
    Json(req): Json<PoolRpcReq>,
) -> Json<PoolRpcResp> {
    Json(dispatch_pool_rpc(&st.pool, req).await)
}

/// Serve sandbox HTTP until graceful `shutdown`. Author: kejiqing
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
        .route("/healthz", get(healthz))
        .route("/healthz/live-report", get(healthz))
        .route("/v1/sandbox/rpc", post(post_sandbox_rpc))
        .route("/v1/pool/rpc", post(post_pool_rpc))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| format!("sandbox http bind {bind}: {e}"))?;
    info!(
        target: "claw_sandbox",
        component = "pool_http",
        phase = "listen",
        bind = %bind,
        "claw-sandbox http listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|e| format!("sandbox http serve: {e}"))
}
