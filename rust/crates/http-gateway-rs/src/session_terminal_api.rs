//! Interactive coding terminal API (`/v1/sessions/.../terminal/*`). Author: kejiqing

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};
use tracing::{info, warn};

use crate::claw_tap_cluster_state::{self, ClawTapClusterHandle};
use crate::client_origin;
use crate::gateway_global_settings;
use crate::gateway_llm_config_sync::LlmRuntimeHandle;
use crate::pool::{
    self, apply_e2b_observe_worker_llm_env, build_proj_bake_script, build_session_attach_script,
    build_start_ttyd_script, gateway_proj_work_dir, gateway_session_home,
    interactive_backend_is_e2b, terminal_ws_connect_url, InteractiveBackendKind, InteractiveLease,
    InteractiveSessionSpec, PoolClients, TtydConnectTarget,
};
use crate::project_config_apply;
use crate::project_config_draft;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct TerminalSessionKey {
    pub proj_id: i64,
    pub session_id: String,
}

#[derive(Debug, Clone)]
pub struct ActiveTerminalSession {
    pub slot_index: usize,
    pub worker_name: Option<String>,
    /// Loopback port for podman; 443 placeholder when e2b public host is used.
    pub ttyd_host_port: u16,
    pub pool_id: String,
    pub backend: InteractiveBackendKind,
    pub e2b_sandbox_id: Option<String>,
    pub e2b_warm_slot: Option<usize>,
    pub e2b_warm_proj_id: Option<i64>,
    pub e2b_session_segment: Option<String>,
    pub e2b_worker_id: Option<String>,
    pub ttyd: TtydConnectTarget,
}

#[derive(Clone, Default)]
pub struct TerminalSessionRegistry {
    inner: Arc<Mutex<HashMap<TerminalSessionKey, ActiveTerminalSession>>>,
}

impl TerminalSessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, key: &TerminalSessionKey) -> Option<ActiveTerminalSession> {
        self.inner.lock().await.get(key).cloned()
    }

    pub async fn insert(&self, key: TerminalSessionKey, value: ActiveTerminalSession) {
        self.inner.lock().await.insert(key, value);
    }

    pub async fn remove(&self, key: &TerminalSessionKey) -> Option<ActiveTerminalSession> {
        self.inner.lock().await.remove(key)
    }

    pub async fn list_for_proj(&self, proj_id: i64) -> Vec<(String, ActiveTerminalSession)> {
        self.inner
            .lock()
            .await
            .iter()
            .filter(|(k, _)| k.proj_id == proj_id)
            .map(|(k, v)| (k.session_id.clone(), v.clone()))
            .collect()
    }

    pub async fn list_all(&self) -> Vec<(TerminalSessionKey, ActiveTerminalSession)> {
        self.inner
            .lock()
            .await
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    pub async fn drain_all(&self) -> Vec<(TerminalSessionKey, ActiveTerminalSession)> {
        let mut guard = self.inner.lock().await;
        let out: Vec<_> = guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        guard.clear();
        out
    }
}

#[derive(Clone)]
pub struct TerminalApiContext {
    pub work_root: PathBuf,
    pub pool_rpc_host_work_root: Option<PathBuf>,
    pub pool_clients: PoolClients,
    pub session_db: Arc<GatewaySessionDb>,
    pub registry: TerminalSessionRegistry,
    pub ttyd_connect_host: String,
    pub pool_runtime_bin: String,
    pub claw_tap_cluster: ClawTapClusterHandle,
    pub llm_runtime: LlmRuntimeHandle,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStartRequest {
    pub proj_id: i64,
    pub session_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStartResponse {
    pub session_id: String,
    pub proj_id: i64,
    pub slot_index: usize,
    pub ttyd_host_port: u16,
    pub ws_path: String,
    pub worker_name: Option<String>,
    /// `true` when an active worker was reused (attach) without pool acquire.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reused_worker: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInventoryEntry {
    pub session_id: String,
    /// `active` = worker + ttyd running; `idle` = disk workspace only.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slot_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ws_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalActiveWorker {
    pub proj_id: i64,
    pub session_id: String,
    pub slot_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalLeasedSlot {
    pub slot_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
    /// `terminal` | `solve` | `orphan`
    pub owner_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proj_id: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalPoolStatus {
    pub slots_max: usize,
    pub slots_idle: usize,
    pub slots_leased: usize,
    pub terminal_count: usize,
    pub solve_count: usize,
    pub orphan_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInventoryResponse {
    pub proj_id: i64,
    pub entries: Vec<TerminalInventoryEntry>,
    pub active_workers: Vec<TerminalActiveWorker>,
    pub leased_slots: Vec<TerminalLeasedSlot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool: Option<TerminalPoolStatus>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalProjQuery {
    pub proj_id: i64,
}

#[derive(Debug, Serialize)]
pub struct TerminalApiErrorBody {
    pub error: String,
}

pub struct TerminalApiError {
    pub status: StatusCode,
    pub message: String,
}

impl TerminalApiError {
    #[must_use]
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for TerminalApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(TerminalApiErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

fn ttyd_connect_host() -> String {
    std::env::var("CLAW_TERMINAL_TTYD_CONNECT_HOST")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

fn terminal_ws_path(session_id: &str, proj_id: i64) -> String {
    format!("/v1/sessions/{session_id}/terminal/ws?projId={proj_id}")
}

fn response_from_active_key(
    session_id: String,
    proj_id: i64,
    active: &ActiveTerminalSession,
    reused_worker: bool,
) -> TerminalStartResponse {
    TerminalStartResponse {
        ws_path: terminal_ws_path(&session_id, proj_id),
        session_id,
        proj_id,
        slot_index: active.slot_index,
        ttyd_host_port: active.ttyd_host_port,
        worker_name: active.worker_name.clone(),
        reused_worker: reused_worker.then_some(true),
    }
}

pub async fn terminal_inventory(
    ctx: TerminalApiContext,
    q: TerminalProjQuery,
) -> Result<Json<TerminalInventoryResponse>, TerminalApiError> {
    let pool = None;
    let leased_slots: Vec<TerminalLeasedSlot> = Vec::new();

    let mut active_workers: Vec<TerminalActiveWorker> = Vec::new();
    for (session_id, active) in ctx.registry.list_for_proj(q.proj_id).await {
        active_workers.push(TerminalActiveWorker {
            proj_id: q.proj_id,
            session_id: session_id.clone(),
            slot_index: active.slot_index,
            worker_name: active.worker_name.clone(),
        });
    }

    let mut entries: Vec<TerminalInventoryEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (session_id, active) in ctx.registry.list_for_proj(q.proj_id).await {
        seen.insert(session_id.clone());
        entries.push(TerminalInventoryEntry {
            session_id: session_id.clone(),
            status: "active".into(),
            slot_index: Some(active.slot_index),
            worker_name: active.worker_name.clone(),
            ws_path: Some(terminal_ws_path(&session_id, q.proj_id)),
        });
    }

    let sessions_dir = gateway_proj_work_dir(&ctx.work_root, q.proj_id)
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?
        .join("sessions");
    if let Ok(mut rd) = tokio::fs::read_dir(&sessions_dir).await {
        while let Ok(Some(ent)) = rd.next_entry().await {
            let Ok(ft) = ent.file_type().await else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let session_id = ent.file_name().to_string_lossy().into_owned();
            if session_id.starts_with('.') || !seen.insert(session_id.clone()) {
                continue;
            }
            entries.push(TerminalInventoryEntry {
                session_id,
                status: "idle".into(),
                slot_index: None,
                worker_name: None,
                ws_path: None,
            });
        }
    }

    entries.sort_by(|a, b| {
        let ar = a.status == "active";
        let br = b.status == "active";
        ar.cmp(&br)
            .reverse()
            .then_with(|| b.session_id.cmp(&a.session_id))
    });
    Ok(Json(TerminalInventoryResponse {
        proj_id: q.proj_id,
        entries,
        active_workers,
        leased_slots,
        pool,
    }))
}

async fn release_active_terminal(
    ctx: &TerminalApiContext,
    _key: &TerminalSessionKey,
    active: &ActiveTerminalSession,
) {
    let lease = active_terminal_to_lease(active);
    if let Err(e) = ctx.pool_clients.stop_interactive_lease(&lease).await {
        warn!(
            target: "claw_gateway_terminal",
            error = %e,
            session_backend = ?active.backend,
            "interactive stop_session failed"
        );
    }
}

fn active_terminal_to_lease(active: &ActiveTerminalSession) -> InteractiveLease {
    InteractiveLease {
        backend: active.backend,
        slot_index: active.slot_index,
        worker_name: active.worker_name.clone(),
        pool_id: active.pool_id.clone(),
        e2b_sandbox_id: active.e2b_sandbox_id.clone(),
        e2b_warm_slot: active.e2b_warm_slot,
        e2b_warm_proj_id: active.e2b_warm_proj_id,
        e2b_session_segment: active.e2b_session_segment.clone(),
        e2b_worker_id: active.e2b_worker_id.clone(),
        ttyd: active.ttyd.clone(),
    }
}

fn lease_to_active_terminal(lease: InteractiveLease) -> ActiveTerminalSession {
    let ttyd_host_port = lease.ttyd.port;
    ActiveTerminalSession {
        slot_index: lease.slot_index,
        worker_name: lease.worker_name,
        ttyd_host_port,
        pool_id: lease.pool_id,
        backend: lease.backend,
        e2b_sandbox_id: lease.e2b_sandbox_id,
        e2b_warm_slot: lease.e2b_warm_slot,
        e2b_warm_proj_id: lease.e2b_warm_proj_id,
        e2b_session_segment: lease.e2b_session_segment,
        e2b_worker_id: lease.e2b_worker_id,
        ttyd: lease.ttyd,
    }
}

/// Force-release every terminal registry entry and kill any remaining leased pool slots.
pub async fn terminal_pool_force_release(
    ctx: TerminalApiContext,
) -> Result<Json<serde_json::Value>, TerminalApiError> {
    let drained = ctx.registry.drain_all().await;
    let mut released = 0usize;
    for (key, active) in &drained {
        release_active_terminal(&ctx, key, active).await;
        released += 1;
    }

    let force_killed = 0usize;
    let _ = force_killed;

    Ok(Json(serde_json::json!({
        "ok": true,
        "releasedRegistry": released,
        "forceKillAttempts": force_killed,
    })))
}

/// Attach to an active worker (instant) or allocate a new worker for an idle session.
pub async fn terminal_attach(
    ctx: TerminalApiContext,
    session_id: String,
    Json(req): Json<TerminalStartRequest>,
) -> Result<Json<TerminalStartResponse>, TerminalApiError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId required",
        ));
    }
    if req.session_id.trim() != session_id {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId in path and body must match",
        ));
    }
    let key = TerminalSessionKey {
        proj_id: req.proj_id,
        session_id: session_id.to_string(),
    };
    if let Some(active) = ctx.registry.get(&key).await {
        info!(
            target: "claw_gateway_terminal",
            session_id = %session_id,
            proj_id = req.proj_id,
            slot_index = active.slot_index,
            "interactive terminal attach (reuse active worker)"
        );
        return Ok(Json(response_from_active_key(
            session_id.to_string(),
            req.proj_id,
            &active,
            true,
        )));
    }
    let mut out = terminal_start(
        ctx,
        Json(TerminalStartRequest {
            proj_id: req.proj_id,
            session_id: session_id.to_string(),
        }),
    )
    .await?;
    out.reused_worker = Some(false);
    Ok(out)
}

pub async fn terminal_start(
    ctx: TerminalApiContext,
    Json(req): Json<TerminalStartRequest>,
) -> Result<Json<TerminalStartResponse>, TerminalApiError> {
    let session_id = req.session_id.trim();
    if session_id.is_empty() {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId required",
        ));
    }
    let key = TerminalSessionKey {
        proj_id: req.proj_id,
        session_id: session_id.to_string(),
    };
    if ctx.registry.get(&key).await.is_some() {
        return Err(TerminalApiError::new(
            StatusCode::CONFLICT,
            "interactive terminal already active for this session",
        ));
    }

    let _ = interactive_backend_is_e2b();
    let session_segment = crate::session_merge::sessions_directory_segment(session_id);
    let session_home = gateway_session_home(&ctx.work_root, req.proj_id, session_id)
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let nas_root = ctx.pool_clients.nas_host_root();
    if ctx.pool_clients.e2b_nas_layout_active() {
        let cluster_id = pool::nas_cluster_id()
            .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        pool::ensure_e2b_proj_nas_roots(&nas_root, &cluster_id, req.proj_id)
            .await
            .map_err(|e| {
                TerminalApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("NAS proj roots: {e}"),
                )
            })?;
    }

    let session_home_rel =
        crate::session_merge::session_home_rel_under_work_root(&ctx.work_root, &session_home)
            .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.detail()))?;
    let client_origin = if session_id.starts_with("ovs-") {
        Some(client_origin::CLIENT_ORIGIN_OVS_CHAT)
    } else {
        None
    };
    let now_ms = crate::persistence::transcript::now_ms();
    if ctx
        .session_db
        .session_exists(session_id, req.proj_id)
        .await
        .map_err(|e| {
            TerminalApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("session registry lookup: {e}"),
            )
        })?
    {
        ctx.session_db
            .touch_updated(session_id, req.proj_id, now_ms)
            .await
            .map_err(|e| {
                TerminalApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("session registry touch: {e}"),
                )
            })?;
    } else {
        ctx.session_db
            .insert_session(
                session_id,
                req.proj_id,
                &session_home_rel,
                now_ms,
                client_origin,
            )
            .await
            .map_err(|e| {
                TerminalApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("session registry insert: {e}"),
                )
            })?;
    }

    let proj_dir = gateway_proj_work_dir(&ctx.work_root, req.proj_id)
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    ctx.pool_clients
        .assert_proj_worker_profile_supported(&ctx.session_db, req.proj_id)
        .await
        .map_err(|e| TerminalApiError::new(StatusCode::BAD_REQUEST, e))?;

    let mut llm_env =
        resolve_terminal_llm_env(&ctx.session_db, &ctx.claw_tap_cluster, &ctx.llm_runtime)
            .await
            .map_err(|e| TerminalApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    llm_env = apply_e2b_observe_worker_llm_env(&ctx.session_db, llm_env)
        .await
        .map_err(|e| TerminalApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;

    let proj_home = proj_dir.join("home");

    let e2b_session_attach_script = Some(build_session_attach_script(&llm_env));
    let e2b_proj_bake_script = Some(
        build_proj_bake_script(&ctx.session_db, req.proj_id)
            .await
            .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?,
    );

    let interactive_backend = ctx
        .pool_clients
        .interactive_backend_for_proj(&ctx.session_db, req.proj_id)
        .await
        .map_err(|e| TerminalApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;

    let spec = InteractiveSessionSpec {
        session_id: session_id.to_string(),
        session_segment: session_segment.clone(),
        proj_id: req.proj_id,
        session_home: session_home.clone(),
        proj_home,
        llm_env,
        ovs_mode: session_id.starts_with("ovs-"),
        start_ttyd_script: build_start_ttyd_script(session_id),
        e2b_session_attach_script,
        e2b_proj_bake_script,
    };

    let lease = interactive_backend
        .start_session(spec)
        .await
        .map_err(|e| TerminalApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;

    let active = lease_to_active_terminal(lease);
    let slot_index = active.slot_index;
    let ttyd_port = active.ttyd_host_port;

    let worker_name = active.worker_name.clone();
    ctx.registry.insert(key.clone(), active).await;

    info!(
        target: "claw_gateway_terminal",
        session_id = %session_id,
        proj_id = req.proj_id,
        slot_index,
        ttyd_host_port = ttyd_port,
        "interactive terminal started"
    );

    Ok(Json(TerminalStartResponse {
        session_id: session_id.to_string(),
        proj_id: req.proj_id,
        slot_index,
        ttyd_host_port: ttyd_port,
        ws_path: terminal_ws_path(session_id, req.proj_id),
        worker_name,
        reused_worker: None,
    }))
}

pub async fn terminal_stop(
    ctx: TerminalApiContext,
    session_id: String,
    q: TerminalProjQuery,
) -> Result<Json<serde_json::Value>, TerminalApiError> {
    let key = TerminalSessionKey {
        proj_id: q.proj_id,
        session_id: session_id.clone(),
    };
    let active = ctx.registry.remove(&key).await.ok_or_else(|| {
        TerminalApiError::new(StatusCode::NOT_FOUND, "no active terminal for session")
    })?;

    release_active_terminal(&ctx, &key, &active).await;

    Ok(Json(
        serde_json::json!({ "ok": true, "sessionId": session_id }),
    ))
}

/// Ensure an interactive worker + ttyd exist for agent/OVS chat (`ovs-{projId}` default).
pub async fn ensure_terminal_active(
    ctx: &TerminalApiContext,
    proj_id: i64,
    session_id: &str,
) -> Result<ActiveTerminalSession, TerminalApiError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(TerminalApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId required",
        ));
    }
    let key = TerminalSessionKey {
        proj_id,
        session_id: session_id.to_string(),
    };
    if let Some(active) = ctx.registry.get(&key).await {
        return Ok(active);
    }
    let _ = terminal_start(
        ctx.clone(),
        Json(TerminalStartRequest {
            proj_id,
            session_id: session_id.to_string(),
        }),
    )
    .await?;
    ctx.registry.get(&key).await.ok_or_else(|| {
        TerminalApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "terminal started but registry missing entry",
        )
    })
}

pub async fn terminal_reattach(
    ctx: TerminalApiContext,
    Json(req): Json<TerminalStartRequest>,
) -> Result<Json<TerminalStartResponse>, TerminalApiError> {
    let _ = ctx
        .registry
        .remove(&TerminalSessionKey {
            proj_id: req.proj_id,
            session_id: req.session_id.clone(),
        })
        .await;
    terminal_start(ctx, Json(req)).await
}

pub async fn terminal_ws_upgrade(
    ctx: TerminalApiContext,
    session_id: String,
    q: TerminalProjQuery,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let key = TerminalSessionKey {
        proj_id: q.proj_id,
        session_id: session_id.clone(),
    };
    let Some(active) = ctx.registry.get(&key).await else {
        return TerminalApiError::new(StatusCode::NOT_FOUND, "no active terminal for session")
            .into_response();
    };
    let ttyd = active.ttyd.clone();
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = proxy_terminal_ws(socket, &ttyd).await {
            warn!(
                target: "claw_gateway_terminal",
                session_id = %session_id,
                error = %e,
                "terminal ws proxy ended with error"
            );
        }
    })
}

async fn proxy_terminal_ws(client: WebSocket, ttyd: &TtydConnectTarget) -> Result<(), String> {
    let url = terminal_ws_connect_url(ttyd);
    let mut req = url
        .as_str()
        .into_client_request()
        .map_err(|e| format!("ws request {url}: {e}"))?;
    req.headers_mut()
        .insert("Sec-WebSocket-Protocol", HeaderValue::from_static("tty"));
    if let Some(host_hdr) = ttyd.proxy_host_header.as_deref() {
        req.headers_mut().insert(
            "Host",
            HeaderValue::from_str(host_hdr).map_err(|e| format!("Host: {e}"))?,
        );
    }
    if let Some(token) = ttyd.traffic_access_token.as_deref() {
        req.headers_mut().insert(
            "X-Access-Token",
            HeaderValue::from_str(token).map_err(|e| format!("X-Access-Token: {e}"))?,
        );
    }
    let (upstream, _) = connect_async(req)
        .await
        .map_err(|e| format!("connect ttyd {url}: {e}"))?;
    let (mut up_tx, mut up_rx) = upstream.split();
    let (mut cli_tx, mut cli_rx) = client.split();

    let client_to_up = async {
        while let Some(msg) = cli_rx.next().await {
            let msg = msg.map_err(|e| format!("client ws: {e}"))?;
            let out = match msg {
                Message::Text(t) => WsMessage::Text(t.to_string().into()),
                Message::Binary(b) => WsMessage::Binary(b),
                Message::Ping(p) => WsMessage::Ping(p),
                Message::Pong(p) => WsMessage::Pong(p),
                Message::Close(_) => WsMessage::Close(None),
            };
            up_tx
                .send(out)
                .await
                .map_err(|e| format!("upstream send: {e}"))?;
        }
        Ok::<(), String>(())
    };

    let up_to_client = async {
        while let Some(msg) = up_rx.next().await {
            let msg = msg.map_err(|e| format!("upstream ws: {e}"))?;
            let out = match msg {
                WsMessage::Text(t) => Message::Text(t.to_string().into()),
                WsMessage::Binary(b) => Message::Binary(b),
                WsMessage::Ping(p) => Message::Ping(p),
                WsMessage::Pong(p) => Message::Pong(p),
                WsMessage::Close(_) => Message::Close(None),
                WsMessage::Frame(_) => continue,
            };
            cli_tx
                .send(out)
                .await
                .map_err(|e| format!("client send: {e}"))?;
        }
        Ok::<(), String>(())
    };

    tokio::select! {
        r = client_to_up => r,
        r = up_to_client => r,
    }
}

async fn materialize_proj_home(
    session_db: &GatewaySessionDb,
    proj_dir: &Path,
    proj_id: i64,
) -> Result<(), String> {
    let row = project_config_draft::row_for_materialize(session_db, proj_id)
        .await
        .map_err(|e| format!("load project_config: {e}"))?;
    let Some(row) = row else {
        tokio::fs::create_dir_all(proj_dir.join("home"))
            .await
            .map_err(|e| format!("mkdir proj home: {e}"))?;
        write_proj_vscode_settings(proj_dir, proj_id).await?;
        return Ok(());
    };
    tokio::fs::create_dir_all(proj_dir.join(".claw"))
        .await
        .map_err(|e| format!("mkdir proj .claw: {e}"))?;
    let scaffold = gateway_global_settings::load_system_prompt_default(session_db)
        .await
        .map_err(|e| format!("load system prompt scaffold: {e}"))?;
    project_config_apply::apply_if_needed(proj_dir, &row, false, &scaffold)
        .await
        .map_err(|e| format!("apply project_config: {e}"))?;
    project_config_apply::apply_interactive_ds_layout_under_home(proj_dir, &row, &scaffold)
        .await
        .map_err(|e| format!("apply interactive /claw_ds layout: {e}"))?;
    project_config_apply::link_claw_compat_symlinks(proj_dir)
        .await
        .map_err(|e| format!("link claw compat symlinks: {e}"))?;
    write_proj_vscode_settings(proj_dir, proj_id).await?;
    Ok(())
}

/// Full PG materialize for OVS workspace (`proj_N/home` + `CLAUDE.md` + interactive layout). OVS path only.
pub async fn materialize_ovs_proj_workspace(
    session_db: &GatewaySessionDb,
    work_root: &Path,
    proj_id: i64,
) -> Result<(), String> {
    let proj_dir = gateway_proj_work_dir(work_root, proj_id)?;
    materialize_proj_home(session_db, &proj_dir, proj_id).await
}

async fn write_proj_vscode_settings(proj_dir: &Path, proj_id: i64) -> Result<(), String> {
    crate::session_ovs_api::ensure_proj_claw_settings(proj_dir, proj_id).await
}

async fn resolve_terminal_llm_env(
    session_db: &GatewaySessionDb,
    cluster_handle: &ClawTapClusterHandle,
    llm_handle: &LlmRuntimeHandle,
) -> Result<BTreeMap<String, String>, String> {
    let (_route, mut env) = claw_tap_cluster_state::resolve_solve_llm_route(
        session_db,
        cluster_handle,
        llm_handle,
        None,
    )
    .await?;
    if let Some(model) = env.remove("CLAW_DEFAULT_MODEL") {
        env.insert(
            "CLAW_DEFAULT_MODEL".to_string(),
            claw_tap_cluster_state::claw_repl_model_name(&model),
        );
    }
    Ok(env)
}

#[must_use]
pub fn terminal_api_context(
    work_root: PathBuf,
    pool_rpc_host_work_root: Option<PathBuf>,
    pool_clients: PoolClients,
    session_db: Arc<GatewaySessionDb>,
    registry: TerminalSessionRegistry,
    pool_runtime_bin: String,
    claw_tap_cluster: ClawTapClusterHandle,
    llm_runtime: LlmRuntimeHandle,
) -> TerminalApiContext {
    TerminalApiContext {
        work_root,
        pool_rpc_host_work_root,
        pool_clients,
        session_db,
        registry,
        ttyd_connect_host: ttyd_connect_host(),
        pool_runtime_bin,
        claw_tap_cluster,
        llm_runtime,
    }
}
