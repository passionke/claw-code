//! Interactive coding terminal API (`/v1/sessions/.../terminal/*`). Author: kejiqing

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use claw_sandbox_protocol::{LeasedSlotInfo, SlotLeaseOwner};
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
use crate::pool::sandbox_orchestrator::worker_isolation_to_sandbox;
use crate::pool::{
    self, proj_work_dir, session_home_under_work_root, terminal_ws_connect_url,
    InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend, InteractiveSessionSpec,
    PoolClients, TtydConnectTarget,
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
    /// Loopback port for podman; 443 placeholder when FC public host is used.
    pub ttyd_host_port: u16,
    pub pool_id: String,
    pub backend: InteractiveBackendKind,
    pub fc_sandbox_id: Option<String>,
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
    pub interactive_backend: Arc<dyn InteractiveSandboxBackend>,
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

const START_TTYD_SH_TERMINAL: &str = r#"
set -e
if ! command -v ttyd >/dev/null 2>&1; then
  echo 'ttyd not installed in worker image' >&2
  exit 127
fi
if [ -f /claw_host_root/.claw/ttyd.pid ]; then
  kill "$(cat /claw_host_root/.claw/ttyd.pid)" 2>/dev/null || true
fi
export HOME=/claw_host_root
export CLAW_PROJECT_CONFIG_ROOT=/claw_ds
export CLAW_GATEWAY_WORK_ROOT=/claw_host_root
export CLAW_DISPLAY_MODE=web
export XDG_CONFIG_HOME=/claw_host_root/.config
export XDG_CACHE_HOME=/claw_host_root/.cache
export XDG_DATA_HOME=/claw_host_root/.local/share
mkdir -p /claw_host_root/.claw/sessions /claw_host_root/.config /claw_host_root/.cache /claw_host_root/.local/share
if [ -f /claw_host_root/.claw/terminal-llm.env ]; then
  set -a
  # shellcheck source=/dev/null
  . /claw_host_root/.claw/terminal-llm.env
  set +a
fi
MODEL="${CLAW_DEFAULT_MODEL:-openai/mimo-v2.5}"
# Non-OVS interactive terminal: cwd stays on session tmpfs (legacy /coding-style).
nohup ttyd -d 1 -i 0.0.0.0 -p 7681 -W -w /claw_host_root \
  /usr/local/bin/claw --allow-broad-cwd --model "$MODEL" \
  >/claw_host_root/.claw/ttyd.log 2>&1 &
echo $! >/claw_host_root/.claw/ttyd.pid
sleep 0.5
kill -0 "$(cat /claw_host_root/.claw/ttyd.pid)" 2>/dev/null
"#;

/// OVS `@claw` interactive REPL: cwd = `/claw_ds` (same tree as OVS `proj_N/home`); session writes under `HOME=/claw_host_root`.
const START_TTYD_SH_OVS: &str = r#"
set -e
if ! command -v ttyd >/dev/null 2>&1; then
  echo 'ttyd not installed in worker image' >&2
  exit 127
fi
if [ -f /claw_host_root/.claw/ttyd.pid ]; then
  kill "$(cat /claw_host_root/.claw/ttyd.pid)" 2>/dev/null || true
fi
export HOME=/claw_host_root
export CLAW_PROJECT_CONFIG_ROOT=/claw_ds
export CLAW_GATEWAY_WORK_ROOT=/claw_host_root
export CLAW_DISPLAY_MODE=web
export XDG_CONFIG_HOME=/claw_host_root/.config
export XDG_CACHE_HOME=/claw_host_root/.cache
export XDG_DATA_HOME=/claw_host_root/.local/share
mkdir -p /claw_host_root/.claw/sessions /claw_host_root/.config /claw_host_root/.cache /claw_host_root/.local/share
if [ -f /claw_host_root/.claw/terminal-llm.env ]; then
  set -a
  # shellcheck source=/dev/null
  . /claw_host_root/.claw/terminal-llm.env
  set +a
fi
MODEL="${CLAW_DEFAULT_MODEL:-openai/mimo-v2.5}"
# OVS alignment: project cwd matches openvscode-server workspace (`proj_N/home` → `/claw_ds`).
nohup ttyd -d 1 -i 0.0.0.0 -p 7681 -W -w /claw_ds \
  /usr/local/bin/claw --allow-broad-cwd --model "$MODEL" \
  >/claw_host_root/.claw/ttyd.log 2>&1 &
echo $! >/claw_host_root/.claw/ttyd.pid
sleep 0.5
kill -0 "$(cat /claw_host_root/.claw/ttyd.pid)" 2>/dev/null
"#;

fn start_ttyd_sh_for_session(session_id: &str) -> &'static str {
    if session_id.starts_with("ovs-") {
        START_TTYD_SH_OVS
    } else {
        START_TTYD_SH_TERMINAL
    }
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

async fn fetch_pool_status(
    ctx: &TerminalApiContext,
) -> Option<(TerminalPoolStatus, Vec<LeasedSlotInfo>)> {
    let sandbox = ctx.pool_clients.sandbox_rpc_client()?;
    let cap = sandbox.capacity().await.ok()?;
    let leased = sandbox.list_leased().await.ok().unwrap_or_default();
    let mut terminal_count = 0usize;
    let mut solve_count = 0usize;
    let mut orphan_count = 0usize;
    for slot in &leased {
        match slot.owner.as_ref() {
            Some(SlotLeaseOwner::Terminal { .. }) => terminal_count += 1,
            Some(SlotLeaseOwner::Solve { .. }) => solve_count += 1,
            None => orphan_count += 1,
        }
    }
    let pool = TerminalPoolStatus {
        slots_max: cap.slots_max,
        slots_idle: cap.slots_idle,
        slots_leased: cap.slots_leased,
        terminal_count,
        solve_count,
        orphan_count,
    };
    Some((pool, leased))
}

fn leased_slot_to_api(slot: &LeasedSlotInfo) -> TerminalLeasedSlot {
    let (owner_kind, session_id, turn_id, proj_id) = match slot.owner.as_ref() {
        Some(SlotLeaseOwner::Terminal {
            session_id,
            proj_id,
        }) => (
            "terminal".to_string(),
            Some(session_id.clone()),
            None,
            Some(*proj_id),
        ),
        Some(SlotLeaseOwner::Solve { turn_id, proj_id }) => (
            "solve".to_string(),
            None,
            Some(turn_id.clone()),
            Some(*proj_id),
        ),
        None => ("orphan".to_string(), None, None, None),
    };
    TerminalLeasedSlot {
        slot_index: slot.slot_index,
        worker_name: slot.worker_name.clone(),
        owner_kind,
        session_id,
        turn_id,
        proj_id,
    }
}

pub async fn terminal_inventory(
    ctx: TerminalApiContext,
    q: TerminalProjQuery,
) -> Result<Json<TerminalInventoryResponse>, TerminalApiError> {
    let (pool, leased_raw) = fetch_pool_status(&ctx)
        .await
        .map(|(p, l)| (Some(p), l))
        .unwrap_or((None, Vec::new()));
    let leased_slots: Vec<TerminalLeasedSlot> = leased_raw.iter().map(leased_slot_to_api).collect();

    let mut active_workers: Vec<TerminalActiveWorker> = leased_raw
        .iter()
        .filter_map(|slot| {
            let SlotLeaseOwner::Terminal {
                session_id,
                proj_id,
            } = slot.owner.as_ref()?
            else {
                return None;
            };
            if *proj_id != q.proj_id {
                return None;
            }
            Some(TerminalActiveWorker {
                proj_id: *proj_id,
                session_id: session_id.clone(),
                slot_index: slot.slot_index,
                worker_name: slot.worker_name.clone(),
            })
        })
        .collect();

    // Gateway registry may still hold ttyd ports for live WS on this process.
    for (session_id, active) in ctx.registry.list_for_proj(q.proj_id).await {
        if active_workers.iter().any(|w| w.session_id == session_id) {
            continue;
        }
        active_workers.push(TerminalActiveWorker {
            proj_id: q.proj_id,
            session_id: session_id.clone(),
            slot_index: active.slot_index,
            worker_name: active.worker_name.clone(),
        });
    }

    let mut entries: Vec<TerminalInventoryEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for slot in &leased_raw {
        let Some(SlotLeaseOwner::Terminal {
            session_id,
            proj_id,
        }) = slot.owner.as_ref()
        else {
            continue;
        };
        if *proj_id != q.proj_id || !seen.insert(session_id.clone()) {
            continue;
        }
        let ws_path = terminal_ws_path(session_id, q.proj_id);
        entries.push(TerminalInventoryEntry {
            session_id: session_id.clone(),
            status: "active".into(),
            slot_index: Some(slot.slot_index),
            worker_name: slot.worker_name.clone(),
            ws_path: Some(ws_path),
        });
    }

    for (session_id, active) in ctx.registry.list_for_proj(q.proj_id).await {
        if !seen.insert(session_id.clone()) {
            continue;
        }
        entries.push(TerminalInventoryEntry {
            session_id: session_id.clone(),
            status: "active".into(),
            slot_index: Some(active.slot_index),
            worker_name: active.worker_name.clone(),
            ws_path: Some(terminal_ws_path(&session_id, q.proj_id)),
        });
    }

    let sessions_dir = proj_work_dir(&ctx.work_root, q.proj_id).join("sessions");
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
    if let Err(e) = ctx.interactive_backend.stop_session(&lease).await {
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
        fc_sandbox_id: active.fc_sandbox_id.clone(),
        ttyd: active.ttyd.clone(),
    }
}

fn lease_to_active_terminal(lease: InteractiveLease) -> ActiveTerminalSession {
    let ttyd_host_port = if lease.ttyd.use_tls {
        lease.ttyd.port
    } else {
        lease.ttyd.port
    };
    ActiveTerminalSession {
        slot_index: lease.slot_index,
        worker_name: lease.worker_name,
        ttyd_host_port,
        pool_id: lease.pool_id,
        backend: lease.backend,
        fc_sandbox_id: lease.fc_sandbox_id,
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

    let mut force_killed = 0usize;
    if let Some(sandbox) = ctx.pool_clients.sandbox_rpc_client() {
        if let Ok(cap) = sandbox.capacity().await {
            for idx in 0..cap.slots_max {
                if sandbox.force_kill(idx).await.is_ok() {
                    force_killed += 1;
                }
            }
        }
    }

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
        materialize_proj_home(
            &ctx.session_db,
            &proj_work_dir(&ctx.work_root, req.proj_id),
            req.proj_id,
        )
        .await
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
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

    let session_home = session_home_under_work_root(&ctx.work_root, req.proj_id, session_id);
    tokio::fs::create_dir_all(&session_home.join(".claw"))
        .await
        .map_err(|e| {
            TerminalApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("mkdir session workspace: {e}"),
            )
        })?;
    pool::ensure_session_tree_owned_for_worker_with_runtime_fallback(
        &ctx.pool_runtime_bin,
        &session_home,
    )
    .await
    .map_err(|e| {
        TerminalApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("session ownership: {e}"),
        )
    })?;

    let session_home_rel = format!(
        "proj_{}/sessions/{}",
        req.proj_id,
        crate::session_merge::sessions_directory_segment(session_id)
    );
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

    let proj_dir = proj_work_dir(&ctx.work_root, req.proj_id);
    materialize_proj_home(&ctx.session_db, &proj_dir, req.proj_id)
        .await
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    ctx.pool_clients
        .assert_proj_worker_isolation_supported(&ctx.session_db, req.proj_id)
        .await
        .map_err(|e| TerminalApiError::new(StatusCode::BAD_REQUEST, e))?;

    let mode = PoolClients::effective_mode_for_proj(&ctx.session_db, req.proj_id).await;
    let isolation = worker_isolation_to_sandbox(mode);

    let llm_env =
        resolve_terminal_llm_env(&ctx.session_db, &ctx.claw_tap_cluster, &ctx.llm_runtime)
            .await
            .map_err(|e| TerminalApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
    write_terminal_llm_env_file(&session_home.join(".claw/terminal-llm.env"), &llm_env)
        .await
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let proj_home = proj_dir.join("home");
    tokio::fs::create_dir_all(&proj_home).await.map_err(|e| {
        TerminalApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("mkdir proj home: {e}"),
        )
    })?;

    let spec = InteractiveSessionSpec {
        session_id: session_id.to_string(),
        proj_id: req.proj_id,
        session_home: session_home.clone(),
        proj_home,
        llm_env,
        ovs_mode: session_id.starts_with("ovs-"),
        sandbox_isolation: isolation,
        start_ttyd_script: start_ttyd_sh_for_session(session_id),
    };

    let lease = ctx
        .interactive_backend
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
        materialize_proj_home(
            &ctx.session_db,
            &proj_work_dir(&ctx.work_root, proj_id),
            proj_id,
        )
        .await
        .map_err(|e| TerminalApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
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
    let active = match ctx.registry.get(&key).await {
        Some(a) => a,
        None => {
            return TerminalApiError::new(StatusCode::NOT_FOUND, "no active terminal for session")
                .into_response();
        }
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
                Message::Binary(b) => WsMessage::Binary(b.into()),
                Message::Ping(p) => WsMessage::Ping(p.into()),
                Message::Pong(p) => WsMessage::Pong(p.into()),
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
                WsMessage::Binary(b) => Message::Binary(b.into()),
                WsMessage::Ping(p) => Message::Ping(p.into()),
                WsMessage::Pong(p) => Message::Pong(p.into()),
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
    let proj_dir = proj_work_dir(work_root, proj_id);
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

async fn write_terminal_llm_env_file(
    path: &Path,
    env: &BTreeMap<String, String>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let body = shell_export_env_file(env);
    tokio::fs::write(path, body)
        .await
        .map_err(|e| format!("write {}: {e}", path.display()))
}

fn shell_export_env_file(env: &BTreeMap<String, String>) -> String {
    let mut out = String::from("# terminal worker LLM env (Admin active LLM + clawTap)\n");
    for (key, value) in env {
        out.push_str("export ");
        out.push_str(key);
        out.push('=');
        out.push_str(&shell_single_quote(value));
        out.push('\n');
    }
    out
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[must_use]
pub fn terminal_api_context(
    work_root: PathBuf,
    pool_rpc_host_work_root: Option<PathBuf>,
    pool_clients: PoolClients,
    session_db: Arc<GatewaySessionDb>,
    registry: TerminalSessionRegistry,
    interactive_backend: Arc<dyn InteractiveSandboxBackend>,
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
        interactive_backend,
        pool_runtime_bin,
        claw_tap_cluster,
        llm_runtime,
    }
}
