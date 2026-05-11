//! Axum gateway: single-binary integration surface (keeps clippy noise localized).
#![allow(clippy::too_many_lines)]
#![allow(clippy::type_complexity)]
#![allow(clippy::result_large_err)]
#![allow(clippy::await_holding_lock)]
#![allow(clippy::format_push_string)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::implicit_clone)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::manual_let_else)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::unnecessary_filter_map)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Extension, Path as AxumPath, Query, Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{AppendHeaders, Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use gateway_solve_turn::run_gateway_solve_turn;
use http_gateway_rs::{session_db, session_merge};
use runtime::load_system_prompt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs;
use tokio::process::Command;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio::task::AbortHandle;
use tokio::time::{interval, timeout, MissedTickBehavior};
use tower_http::trace::TraceLayer;
use tracing::field::Empty;
use tracing::{info, warn};
use uuid::Uuid;

mod gateway_logging;
mod pool;
mod solve_pool;

fn default_system_date() -> String {
    match option_env!("BUILD_DATE") {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => current_utc_date(),
    }
}

/// Session id from `claw-session-id` (fallback `x-request-id`) or generated. kejiqing
#[derive(Clone)]
struct HttpRequestId(pub String);

#[derive(Clone)]
struct RunSolveContext {
    request_id: String,
    task_id: Option<String>,
    /// When true, do not read/write the gateway session `SQLite` (e.g. internal biz report solve).
    skip_session_db: bool,
}

#[derive(Clone)]
struct AppState {
    tasks: Arc<Mutex<HashMap<String, TaskInner>>>,
    injected_mcp: Arc<Mutex<HashMap<i64, HashMap<String, Value>>>>,
    ds_locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
    /// Serialize solve per `(ds_id, session_id)` for transcript + workspace safety.
    session_solve_locks: Arc<Mutex<HashMap<(i64, String), Arc<Mutex<()>>>>>,
    session_db: Arc<session_db::GatewaySessionDb>,
    cfg: Arc<GatewayConfig>,
    /// When using `docker_pool` / `podman_pool`, active async task id → pool + slot for cancel.
    docker_slots: Arc<Mutex<HashMap<String, (Arc<dyn pool::PoolOps + Send + Sync>, usize)>>>,
    docker_pool: Option<Arc<dyn pool::PoolOps + Send + Sync>>,
    /// Serialize git and working-tree reads/writes on the shared `.claw-code-projects` clone. kejiqing
    projects_git_mirror_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SolveIsolation {
    InProcess,
    DockerPool,
    PodmanPool,
}

impl SolveIsolation {
    fn from_env() -> Self {
        // Default product mode is Podman container pool; set CLAW_SOLVE_ISOLATION=inprocess to disable.
        match std::env::var("CLAW_SOLVE_ISOLATION")
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_default()
            .as_str()
        {
            "inprocess" => Self::InProcess,
            "docker_pool" => Self::DockerPool,
            _ => Self::PodmanPool,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::InProcess => "inprocess",
            Self::DockerPool => "docker_pool",
            Self::PodmanPool => "podman_pool",
        }
    }
}

#[derive(Clone)]
struct GatewayConfig {
    solve_isolation: SolveIsolation,
    claw_bin: String,
    work_root: PathBuf,
    /// Host `CLAW_WORK_ROOT` equivalent when the gateway is containerized and uses [`pool::PoolRpcClient`].
    pool_rpc_host_work_root: Option<PathBuf>,
    /// `CLAW_POOL_DAEMON_TCP` (`host:port`) when using TCP to host daemon.
    pool_rpc_tcp: Option<String>,
    /// `CLAW_POOL_DAEMON_SOCKET` when using Unix RPC (optional).
    pool_rpc_unix_socket: Option<String>,
    /// True when pool RPC goes to out-of-process daemon (TCP or Unix).
    pool_rpc_remote: bool,
    ds_registry_path: PathBuf,
    default_timeout_seconds: u64,
    default_max_iterations: usize,
    default_http_mcp_name: Option<String>,
    default_http_mcp_url: Option<String>,
    default_http_mcp_transport: String,
    config_mcp_servers: HashMap<String, Value>,
    allowed_tools: Vec<String>,
    /// Remote URL for `claw-code-projects` mirror (SSH or HTTPS; no embedded token).
    projects_git_url: String,
    projects_git_branch: String,
    /// Passed to `git commit --author`.
    projects_git_author: String,
    /// When set with an `https://` `projects_git_url`, used for clone/pull/push (GitHub: `x-access-token`).
    projects_git_token: Option<String>,
    /// When set, periodically `git pull` the mirror and refresh each `ds_*/home` when that ds lock is idle (multi-node). kejiqing
    projects_git_ds_home_poll_interval_secs: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SolveRequest {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "userPrompt")]
    user_prompt: String,
    /// When set, continue an existing gateway session for this `dsId` (must exist in session DB).
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
    model: Option<String>,
    #[serde(rename = "timeoutSeconds")]
    timeout_seconds: Option<u64>,
    #[serde(rename = "extraSession")]
    extra_session: Option<Value>,
    #[serde(rename = "allowedTools")]
    allowed_tools: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct SolveResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    request_id: String,
    /// Relative to `CLAW_WORK_ROOT` (matches DB `gateway_sessions.session_home`). kejiqing
    #[serde(rename = "sessionHomeRel")]
    session_home_rel: String,
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    #[serde(rename = "durationMs")]
    duration_ms: i64,
    #[serde(rename = "clawExitCode")]
    claw_exit_code: i32,
    #[serde(rename = "outputText")]
    output_text: String,
    #[serde(rename = "outputJson")]
    output_json: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SolveAsyncResponse {
    #[serde(rename = "taskId")]
    task_id: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    request_id: String,
    status: String,
    #[serde(rename = "pollUrl")]
    poll_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InitRequest {
    #[serde(rename = "dsId")]
    ds_id: i64,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateProjectClaudeRequest {
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpsertProjectSkillRequest {
    #[serde(rename = "skillName")]
    skill_name: String,
    #[serde(rename = "skillContent")]
    skill_content: String,
}

#[derive(Debug, Serialize)]
struct InitResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    initialized: bool,
}

#[derive(Debug, Serialize)]
struct ProjectClaudeResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    path: String,
    exists: bool,
    content: String,
}

#[derive(Debug, Serialize)]
struct GitSyncResponse {
    repo: String,
    branch: String,
    #[serde(rename = "commitId")]
    commit_id: String,
    pushed: bool,
}

#[derive(Debug, Serialize)]
struct ProjectSkillResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "skillName")]
    skill_name: String,
    #[serde(rename = "skillPath")]
    skill_path: String,
    created: bool,
    updated: bool,
    #[serde(rename = "bytesWritten")]
    bytes_written: usize,
    #[serde(rename = "workDir")]
    work_dir: String,
    #[serde(rename = "gitSync")]
    git_sync: GitSyncResponse,
}

#[derive(Debug, Serialize)]
struct EffectivePromptResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    sections: Vec<String>,
    message: String,
}

/// In-memory task row plus a handle to abort the async worker (not serialized). kejiqing
struct TaskInner {
    record: TaskRecord,
    /// Present while `queued` / `running`; cleared when the worker finishes or after cancel.
    cancel: Option<AbortHandle>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct TaskRecord {
    #[serde(rename = "taskId")]
    task_id: String,
    #[serde(rename = "sessionId")]
    session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    request_id: String,
    status: String,
    #[serde(rename = "createdAtMs")]
    created_at_ms: i64,
    #[serde(rename = "startedAtMs")]
    started_at_ms: Option<i64>,
    #[serde(rename = "finishedAtMs")]
    finished_at_ms: Option<i64>,
    result: Option<SolveResponse>,
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ProbeQuery {
    #[serde(rename = "probe_timeout_seconds")]
    probe_timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct BizAdviceReportQuery {
    task_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BizAdviceReportResponse {
    #[serde(rename = "taskId")]
    task_id: String,
    #[serde(rename = "sourceRequestId")]
    source_request_id: String,
    #[serde(rename = "sourceDsId")]
    source_ds_id: i64,
    #[serde(rename = "sourceStatus")]
    source_status: String,
    #[serde(rename = "reportText")]
    report_text: String,
    #[serde(rename = "reportJson")]
    report_json: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct DeleteQuery {
    server_names: Option<String>,
    #[serde(rename = "probe_timeout_seconds")]
    probe_timeout_seconds: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InjectMcpRequest {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "mcpServers")]
    mcp_servers: HashMap<String, Value>,
    replace: Option<bool>,
}

#[derive(Debug, Serialize)]
struct McpResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    request_id: String,
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "injectedServerNames")]
    injected_server_names: Vec<String>,
    loaded: bool,
    #[serde(rename = "missingServers")]
    missing_servers: Vec<String>,
    #[serde(rename = "configuredServers")]
    configured_servers: i64,
    status: String,
    #[serde(rename = "mcpReport")]
    mcp_report: Value,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn detail(&self) -> &str {
        &self.message
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "detail": self.message }))).into_response()
    }
}

fn session_routing_error(e: session_merge::SessionRoutingError) -> ApiError {
    let status = match e {
        session_merge::SessionRoutingError::AbsNotUnderWorkRoot => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        _ => StatusCode::BAD_REQUEST,
    };
    ApiError::new(status, e.detail())
}

async fn inject_http_request_id(mut req: Request, next: Next) -> Response {
    let id_claw = req
        .headers()
        .get("claw-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let id_xreq = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let (id, kind) = if let Some(id) = id_claw {
        (id, session_merge::HttpRequestIdKind::FromClientHeader)
    } else if let Some(id) = id_xreq {
        (id, session_merge::HttpRequestIdKind::FromClientHeader)
    } else {
        (
            Uuid::new_v4().simple().to_string(),
            session_merge::HttpRequestIdKind::Generated,
        )
    };
    req.extensions_mut().insert(HttpRequestId(id.clone()));
    req.extensions_mut().insert(kind);
    let mut res = next.run(req).await;
    let xrid = header::HeaderName::from_static("x-request-id");
    let csid = header::HeaderName::from_static("claw-session-id");
    // Handlers such as `/v1/solve` set these from the merged effective session id; do not overwrite.
    if !res.headers().contains_key(&xrid) {
        if let Ok(value) = HeaderValue::from_str(&id) {
            res.headers_mut().insert(xrid, value);
        }
    }
    if !res.headers().contains_key(&csid) {
        if let Ok(value) = HeaderValue::from_str(&id) {
            res.headers_mut().insert(csid, value);
        }
    }
    res
}

async fn get_session_solve_lock(state: &AppState, ds_id: i64, session_id: &str) -> Arc<Mutex<()>> {
    let mut locks = state.session_solve_locks.lock().await;
    locks
        .entry((ds_id, session_id.to_string()))
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn session_db_err(e: &sqlx::Error) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("gateway session database error: {e}"),
    )
}

/// Directory used for pool bind mounts (`worker -v …:/claw_host_root`), as seen by the gateway process.
/// In the Podman compose stack this is the container path `/var/lib/claw/workspace` (same as `CLAW_WORK_ROOT`).
/// If `CLAW_POOL_WORK_ROOT_HOST` points at a path that does not exist in this filesystem (e.g. a macOS
/// `/Users/...` path inside a Linux gateway container), we fall back to `work_root`. Author: kejiqing
fn pool_host_bind_root(work_root: &Path, isolation: SolveIsolation) -> PathBuf {
    match isolation {
        SolveIsolation::InProcess => work_root.to_path_buf(),
        SolveIsolation::DockerPool | SolveIsolation::PodmanPool => {
            if let Ok(raw) = std::env::var("CLAW_POOL_WORK_ROOT_HOST") {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    let p = PathBuf::from(trimmed);
                    if p.exists() {
                        return p;
                    }
                    warn!(
                        target: "claw_gateway_orchestration",
                        component = "startup",
                        phase = "pool_host_bind_root_fallback",
                        configured = %trimmed,
                        fallback = %work_root.display(),
                        "CLAW_POOL_WORK_ROOT_HOST not found in this filesystem; using CLAW_WORK_ROOT"
                    );
                }
            }
            work_root.to_path_buf()
        }
    }
}

fn mandatory_nonempty_env(var: &'static str) -> String {
    if let Ok(value) = std::env::var(var) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            eprintln!(
                "http-gateway-rs: {var} is set but empty; set a non-empty value (e.g. in deploy .env)."
            );
            std::process::exit(1);
        }
        trimmed.to_string()
    } else {
        eprintln!(
            "http-gateway-rs: {var} is required for project Git sync; set it in the environment (see repo root .env.example)."
        );
        std::process::exit(1);
    }
}

fn validate_projects_git_at_startup(url: &str, token: Option<&str>) {
    if url.starts_with("https://") {
        let rest = url.trim_start_matches("https://");
        let has_userinfo = rest.contains('@');
        let has_token = token.is_some_and(|t| !t.trim().is_empty());
        if !has_userinfo && !has_token {
            eprintln!(
                "http-gateway-rs: CLAW_PROJECTS_GIT_URL is HTTPS without embedded credentials (no userinfo before host) and CLAW_PROJECTS_GIT_TOKEN is unset or empty; set CLAW_PROJECTS_GIT_TOKEN or use an SSH URL."
            );
            std::process::exit(1);
        }
    }
}

#[tokio::main]
async fn main() {
    let solve_isolation = SolveIsolation::from_env();
    let work_root = PathBuf::from(
        std::env::var("CLAW_WORK_ROOT").unwrap_or_else(|_| "/tmp/claw-workspace".to_string()),
    );
    gateway_logging::init(&work_root);
    let file_log = gateway_logging::resolved_file_log_dir(&work_root);
    info!(
        target: "claw_gateway_orchestration",
        component = "startup",
        phase = "process_boot",
        work_root = %work_root.display(),
        solve_isolation = solve_isolation.as_str(),
        file_log_dir = file_log.as_ref().map(|p| p.display().to_string()),
        file_log_enabled = file_log.is_some(),
        stdout_json_forced_for_file_sink = file_log.is_some(),
        http_addr = %std::env::var("CLAW_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
        "http-gateway-rs tracing ready; when file_log_enabled, stdout is JSON too (same subscriber layers)"
    );
    let pool_binding_root = pool_host_bind_root(&work_root, solve_isolation);
    if matches!(
        solve_isolation,
        SolveIsolation::DockerPool | SolveIsolation::PodmanPool
    ) {
        info!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "pool_host_paths",
            work_root = %work_root.display(),
            pool_host_bind_root = %pool_binding_root.display(),
            "container pool uses pool_host_bind_root on the runtime host for worker -v mounts"
        );
    }
    let pool_rpc_host_work_root = std::env::var("CLAW_POOL_RPC_HOST_WORK_ROOT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);

    let pool_daemon_tcp = std::env::var("CLAW_POOL_DAEMON_TCP")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let pool_daemon_socket = std::env::var("CLAW_POOL_DAEMON_SOCKET")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let pool_rpc_tcp_cfg = pool_daemon_tcp.clone();
    let pool_rpc_unix_cfg = pool_daemon_socket.clone();

    let docker_pool: Option<Arc<dyn pool::PoolOps + Send + Sync>> = match solve_isolation {
        SolveIsolation::DockerPool | SolveIsolation::PodmanPool => {
            if let Some(ref tcp_addr) = pool_daemon_tcp {
                if pool_rpc_host_work_root.is_none() {
                    warn!(
                        target: "claw_gateway_orchestration",
                        component = "startup",
                        phase = "pool_rpc_missing_host_root",
                        "CLAW_POOL_DAEMON_TCP is set but CLAW_POOL_RPC_HOST_WORK_ROOT is empty; acquire paths may not match the host daemon"
                    );
                }
                let client = pool::PoolRpcClient::new_tcp(tcp_addr.clone());
                Some(Arc::new(client))
            } else if let Some(ref sock_path) = pool_daemon_socket {
                if pool_rpc_host_work_root.is_none() {
                    warn!(
                        target: "claw_gateway_orchestration",
                        component = "startup",
                        phase = "pool_rpc_missing_host_root",
                        "CLAW_POOL_DAEMON_SOCKET is set but CLAW_POOL_RPC_HOST_WORK_ROOT is empty; acquire paths may not match the host daemon"
                    );
                }
                let client = pool::PoolRpcClient::new(PathBuf::from(sock_path));
                Some(Arc::new(client))
            } else {
                let podman = matches!(solve_isolation, SolveIsolation::PodmanPool);
                let p = pool::DockerPoolManager::try_from_env(podman, &pool_binding_root)
                    .unwrap_or_else(|e| {
                        let runtime = if podman { "Podman" } else { "Docker" };
                        eprintln!("http-gateway-rs: invalid {runtime} pool configuration: {e}");
                        std::process::exit(1);
                    });
                pool::DockerPoolManager::schedule_warm(&p);
                Some(Arc::new(pool::LocalPoolOps(p)))
            }
        }
        SolveIsolation::InProcess => None,
    };

    let projects_git_url = mandatory_nonempty_env("CLAW_PROJECTS_GIT_URL");
    let projects_git_branch = mandatory_nonempty_env("CLAW_PROJECTS_GIT_BRANCH");
    let projects_git_author = mandatory_nonempty_env("CLAW_PROJECTS_GIT_AUTHOR");
    let projects_git_token = std::env::var("CLAW_PROJECTS_GIT_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    validate_projects_git_at_startup(&projects_git_url, projects_git_token.as_deref());

    let cfg = GatewayConfig {
        solve_isolation,
        claw_bin: std::env::var("CLAW_BIN").unwrap_or_else(|_| "claw".to_string()),
        work_root,
        pool_rpc_host_work_root,
        pool_rpc_tcp: pool_rpc_tcp_cfg,
        pool_rpc_unix_socket: pool_rpc_unix_cfg,
        pool_rpc_remote: pool_daemon_tcp.is_some() || pool_daemon_socket.is_some(),
        ds_registry_path: PathBuf::from(std::env::var("CLAW_DS_REGISTRY").unwrap_or_else(|_| {
            "third_party/claw-http-gateway/http_gateway/config/datasources.example.yaml".to_string()
        })),
        default_timeout_seconds: std::env::var("CLAW_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120),
        default_max_iterations: std::env::var("CLAW_MAX_ITERATIONS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(64),
        default_http_mcp_name: std::env::var("CLAW_DEFAULT_HTTP_MCP_NAME")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        default_http_mcp_url: std::env::var("CLAW_DEFAULT_HTTP_MCP_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        default_http_mcp_transport: std::env::var("CLAW_DEFAULT_HTTP_MCP_TRANSPORT")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| v == "http" || v == "sse")
            .unwrap_or_else(|| "http".to_string()),
        config_mcp_servers: load_mcp_servers_from_claw_config(),
        allowed_tools: parse_allowed_tools(std::env::var("CLAW_ALLOWED_TOOLS").ok()),
        projects_git_url,
        projects_git_branch,
        projects_git_author,
        projects_git_token,
        projects_git_ds_home_poll_interval_secs: std::env::var(
            "CLAW_PROJECTS_GIT_DS_HOME_POLL_INTERVAL_SECS",
        )
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0),
    };
    let session_db = Arc::new(
        session_db::GatewaySessionDb::open(&cfg.work_root)
            .await
            .unwrap_or_else(|e| {
                eprintln!("http-gateway-rs: failed to open gateway session SQLite: {e}");
                std::process::exit(1);
            }),
    );
    info!(
        target: "claw_gateway_orchestration",
        component = "startup",
        phase = "session_db",
        session_db_path = %session_db.path().display(),
        "gateway session SQLite ready (CLAW_GATEWAY_SESSION_DB or work_root/gateway-sessions.sqlite)"
    );
    let state = AppState {
        tasks: Arc::new(Mutex::new(HashMap::new())),
        injected_mcp: Arc::new(Mutex::new(HashMap::new())),
        ds_locks: Arc::new(Mutex::new(HashMap::new())),
        session_solve_locks: Arc::new(Mutex::new(HashMap::new())),
        session_db,
        cfg: Arc::new(cfg),
        docker_slots: Arc::new(Mutex::new(HashMap::new())),
        docker_pool,
        projects_git_mirror_lock: Arc::new(Mutex::new(())),
    };

    if let Some(secs) = state.cfg.projects_git_ds_home_poll_interval_secs {
        let poller_state = state.clone();
        tokio::spawn(async move { projects_git_ds_home_poll_loop(poller_state, secs).await });
        info!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "projects_git_poll",
            interval_secs = secs,
            "background ds home sync from mirror enabled"
        );
    }

    let app = Router::new()
        .route("/", get(root))
        .route("/docs", get(docs))
        .route("/dos", get(docs))
        .route("/openapi.json", get(openapi))
        .route("/healthz", get(healthz))
        .route("/v1/init", post(init_workspace))
        .route("/v1/solve", post(solve))
        .route("/v1/solve_async", post(solve_async))
        .route("/v1/tasks/{task_id}", get(get_task))
        .route("/v1/tasks/{task_id}/cancel", post(cancel_task))
        .route("/v1/biz_advice_report", get(get_biz_advice_report))
        .route(
            "/v1/project/claude/{ds_id}",
            get(get_project_claude_md).post(update_project_claude_md),
        )
        .route("/v1/project/skills/{ds_id}", post(upsert_project_skill))
        .route(
            "/v1/project/prompt/{ds_id}/effective",
            get(get_effective_prompt).post(post_effective_prompt),
        )
        .route("/v1/mcp/inject", post(inject_mcp))
        .route("/v1/mcp/injected/{ds_id}", get(get_injected_mcp))
        .route("/v1/mcp/injected/{ds_id}", delete(delete_injected_mcp))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &http::Request<axum::body::Body>| {
                    let request_id = request
                        .extensions()
                        .get::<HttpRequestId>()
                        .map_or("-", |h| h.0.as_str());
                    tracing::info_span!(
                        "http_request",
                        http.method = %request.method(),
                        http.uri = %request.uri(),
                        http.version = ?request.version(),
                        request_id = %request_id,
                        http.status_code = Empty,
                        latency_ms = Empty,
                    )
                })
                .on_response(
                    |response: &http::Response<axum::body::Body>,
                     latency: std::time::Duration,
                     span: &tracing::Span| {
                        span.record(
                            "http.status_code",
                            tracing::field::display(response.status().as_u16()),
                        );
                        span.record("latency_ms", latency.as_millis() as u64);
                    },
                ),
        )
        .layer(middleware::from_fn(inject_http_request_id))
        .with_state(state);

    let addr = std::env::var("CLAW_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    info!("http gateway rs listening on {}", addr);
    let shutdown = async {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
                tokio::select! {
                    res = tokio::signal::ctrl_c() => {
                        if res.is_ok() {
                            info!(phase = "shutdown", "http gateway received SIGINT");
                        }
                    }
                    _ = sigterm.recv() => {
                        info!(phase = "shutdown", "http gateway received SIGTERM");
                    }
                }
            } else if tokio::signal::ctrl_c().await.is_ok() {
                info!(phase = "shutdown", "http gateway received SIGINT");
            }
        }
        #[cfg(not(unix))]
        if tokio::signal::ctrl_c().await.is_ok() {
            info!(phase = "shutdown", "http gateway received SIGINT");
        }
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("start axum");
}

async fn root() -> Html<&'static str> {
    Html("<h3>claw gateway rs</h3><p>Open <a href=\"/docs\">/docs</a> to view all endpoints.</p>")
}

async fn docs() -> Html<String> {
    let rows = [
        ("GET", "/", "Gateway welcome page"),
        ("GET", "/docs", "API docs page"),
        ("GET", "/dos", "Alias of /docs"),
        ("GET", "/openapi.json", "OpenAPI-style JSON"),
        ("GET", "/healthz", "Health check"),
        ("POST", "/v1/init", "Initialize workspace for dsId"),
        ("POST", "/v1/solve", "Run sync solve"),
        ("POST", "/v1/solve_async", "Create async solve task"),
        ("GET", "/v1/tasks/{task_id}", "Get async task status"),
        (
            "POST",
            "/v1/tasks/{task_id}/cancel",
            "Cancel a queued or running async solve task",
        ),
        (
            "GET",
            "/v1/biz_advice_report?task_id=xx",
            "Generate cleaned final report from async task output",
        ),
        (
            "GET",
            "/v1/project/claude/{ds_id}",
            "Get project CLAUDE.md for ds",
        ),
        (
            "POST",
            "/v1/project/claude/{ds_id}",
            "Update project CLAUDE.md for ds",
        ),
        (
            "POST",
            "/v1/project/skills/{ds_id}",
            "Create or update project skill for ds",
        ),
        (
            "GET",
            "/v1/project/prompt/{ds_id}/effective",
            "Get effective system prompt for ds",
        ),
        (
            "POST",
            "/v1/project/prompt/{ds_id}/effective",
            "Reload and get effective system prompt for ds",
        ),
        ("POST", "/v1/mcp/inject", "Inject MCP servers"),
        ("GET", "/v1/mcp/injected/{ds_id}", "Get MCP servers for ds"),
        (
            "DELETE",
            "/v1/mcp/injected/{ds_id}",
            "Delete MCP servers for ds",
        ),
    ];
    let mut body = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>claw gateway docs</title></head><body>\
         <h2>claw gateway rs - API docs</h2>\
         <p>OpenAPI JSON: <a href=\"/openapi.json\">/openapi.json</a></p>\
         <table border=\"1\" cellpadding=\"8\" cellspacing=\"0\">\
         <tr><th>Method</th><th>Path</th><th>Description</th></tr>",
    );
    for (method, path, desc) in rows {
        body.push_str(&format!(
            "<tr><td>{}</td><td><code>{}</code></td><td>{}</td></tr>",
            method, path, desc
        ));
    }
    body.push_str("</table></body></html>");
    Html(body)
}

async fn openapi() -> Json<Value> {
    Json(json!({
        "openapi": "3.0.0",
        "info": {
            "title": "claw gateway rs",
            "version": "0.1.0"
        },
        "components": {
            "schemas": {
                "SolveRequest": {
                    "type": "object",
                    "required": ["dsId", "userPrompt"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64", "minimum": 1, "description": "Datasource ID" },
                        "userPrompt": { "type": "string", "minLength": 1, "description": "User prompt text" },
                        "sessionId": { "type": "string", "nullable": true, "description": "Optional: continue an existing gateway session for this dsId (must exist in gateway session DB). When set, conflicts with an explicit claw-session-id / x-request-id header yield 400." },
                        "model": { "type": "string", "nullable": true, "description": "Optional model override" },
                        "timeoutSeconds": { "type": "integer", "format": "int64", "nullable": true, "description": "Optional timeout in seconds" },
                        "allowedTools": {
                            "type": "array",
                            "nullable": true,
                            "description": "Optional per-request tool allowlist/patterns; applied to both /v1/solve and /v1/solve_async.",
                            "items": { "type": "string" }
                        }
                    }
                },
                "InitRequest": {
                    "type": "object",
                    "required": ["dsId"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64", "minimum": 1, "description": "Datasource ID" }
                    }
                },
                "InitResponse": {
                    "type": "object",
                    "required": ["dsId", "workDir", "initialized"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64" },
                        "workDir": { "type": "string" },
                        "initialized": { "type": "boolean" }
                    }
                },
                "UpdateProjectClaudeRequest": {
                    "type": "object",
                    "required": ["content"],
                    "properties": {
                        "content": { "type": "string", "description": "CLAUDE.md content" }
                    }
                },
                "ProjectClaudeResponse": {
                    "type": "object",
                    "required": ["dsId", "workDir", "path", "exists", "content"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64" },
                        "workDir": { "type": "string" },
                        "path": { "type": "string" },
                        "exists": { "type": "boolean" },
                        "content": { "type": "string" }
                    }
                },
                "UpsertProjectSkillRequest": {
                    "type": "object",
                    "required": ["skillName", "skillContent"],
                    "properties": {
                        "skillName": { "type": "string", "description": "Skill name; allowed chars: [a-zA-Z0-9._-]" },
                        "skillContent": { "type": "string", "description": "Content written into SKILL.md (same as Skill tool / CLI)" }
                    }
                },
                "GitSyncResponse": {
                    "type": "object",
                    "required": ["repo", "branch", "commitId", "pushed"],
                    "properties": {
                        "repo": { "type": "string" },
                        "branch": { "type": "string" },
                        "commitId": { "type": "string" },
                        "pushed": { "type": "boolean" }
                    }
                },
                "ProjectSkillResponse": {
                    "type": "object",
                    "required": ["dsId", "skillName", "skillPath", "created", "updated", "bytesWritten", "workDir", "gitSync"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64" },
                        "skillName": { "type": "string" },
                        "skillPath": { "type": "string" },
                        "created": { "type": "boolean" },
                        "updated": { "type": "boolean" },
                        "bytesWritten": { "type": "integer", "format": "int64" },
                        "workDir": { "type": "string" },
                        "gitSync": { "$ref": "#/components/schemas/GitSyncResponse" }
                    }
                },
                "EffectivePromptResponse": {
                    "type": "object",
                    "required": ["dsId", "workDir", "sections", "message"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64" },
                        "workDir": { "type": "string" },
                        "sections": { "type": "array", "items": { "type": "string" } },
                        "message": { "type": "string" }
                    }
                },
                "SolveResponse": {
                    "type": "object",
                    "required": ["sessionId", "requestId", "sessionHomeRel", "dsId", "workDir", "durationMs", "clawExitCode", "outputText"],
                    "properties": {
                        "sessionId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "sessionHomeRel": { "type": "string", "description": "Under CLAW_WORK_ROOT; same as gateway_sessions.session_home. New sessions use ds_{id}/sessions/<segment> where <segment> equals sessionId when it is a safe single path component; otherwise a deterministic 32-hex segment." },
                        "dsId": { "type": "integer", "format": "int64" },
                        "workDir": { "type": "string" },
                        "durationMs": { "type": "integer", "format": "int64" },
                        "clawExitCode": { "type": "integer", "format": "int32" },
                        "outputText": { "type": "string" },
                        "outputJson": { "type": "object", "nullable": true }
                    }
                },
                "SolveAsyncResponse": {
                    "type": "object",
                    "required": ["taskId", "sessionId", "requestId", "status", "pollUrl"],
                    "properties": {
                        "taskId": { "type": "string" },
                        "sessionId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "status": { "type": "string" },
                        "pollUrl": { "type": "string" }
                    }
                },
                "InjectMcpRequest": {
                    "type": "object",
                    "required": ["dsId", "mcpServers"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64", "minimum": 1 },
                        "mcpServers": { "type": "object", "description": "MCP server config map" },
                        "replace": { "type": "boolean", "nullable": true }
                    }
                },
                "McpResponse": {
                    "type": "object",
                    "required": ["sessionId", "requestId", "dsId", "injectedServerNames", "loaded", "missingServers", "configuredServers", "status", "mcpReport"],
                    "properties": {
                        "sessionId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "dsId": { "type": "integer", "format": "int64" },
                        "injectedServerNames": { "type": "array", "items": { "type": "string" } },
                        "loaded": { "type": "boolean" },
                        "missingServers": { "type": "array", "items": { "type": "string" } },
                        "configuredServers": { "type": "integer", "format": "int64" },
                        "status": { "type": "string" },
                        "mcpReport": { "type": "object" }
                    }
                },
                "TaskRecord": {
                    "type": "object",
                    "required": ["taskId", "sessionId", "requestId", "status", "createdAtMs"],
                    "properties": {
                        "taskId": { "type": "string" },
                        "sessionId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "status": { "type": "string" },
                        "createdAtMs": { "type": "integer", "format": "int64" },
                        "startedAtMs": { "type": "integer", "format": "int64", "nullable": true },
                        "finishedAtMs": { "type": "integer", "format": "int64", "nullable": true },
                        "result": { "$ref": "#/components/schemas/SolveResponse", "nullable": true },
                        "error": { "type": "object", "nullable": true }
                    }
                },
                "BizAdviceReportResponse": {
                    "type": "object",
                    "required": ["taskId", "sourceRequestId", "sourceDsId", "sourceStatus", "reportText"],
                    "properties": {
                        "taskId": { "type": "string" },
                        "sourceRequestId": { "type": "string" },
                        "sourceDsId": { "type": "integer", "format": "int64" },
                        "sourceStatus": { "type": "string" },
                        "reportText": { "type": "string" },
                        "reportJson": { "type": "object", "nullable": true }
                    }
                }
            }
        },
        "paths": {
            "/": { "get": { "summary": "Gateway welcome page" } },
            "/docs": { "get": { "summary": "API docs page" } },
            "/dos": { "get": { "summary": "Alias of /docs" } },
            "/openapi.json": { "get": { "summary": "OpenAPI-style JSON" } },
            "/healthz": {
                "get": {
                    "summary": "Health check",
                    "responses": {
                        "200": { "description": "Gateway health and runtime settings", "content": { "application/json": { "schema": { "type": "object" } } } }
                    }
                }
            },
            "/v1/init": {
                "post": {
                    "summary": "Initialize workspace for dsId",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/InitRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Workspace initialized", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/InitResponse" } } } }
                    }
                }
            },
            "/v1/solve": {
                "post": {
                    "summary": "Run sync solve",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SolveRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Solve finished", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SolveResponse" } } } },
                        "400": { "description": "Unknown sessionId for continuation or header/body session conflict" }
                    }
                }
            },
            "/v1/solve_async": {
                "post": {
                    "summary": "Create async solve task",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SolveRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Task created", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SolveAsyncResponse" } } } },
                        "400": { "description": "Unknown sessionId for continuation" },
                        "409": { "description": "Same sessionId already has a queued or running async task" }
                    }
                }
            },
            "/v1/tasks/{task_id}": {
                "get": {
                    "summary": "Get async task status",
                    "parameters": [
                        { "name": "task_id", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": { "description": "Task status", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TaskRecord" } } } }
                    }
                }
            },
            "/v1/tasks/{task_id}/cancel": {
                "post": {
                    "summary": "Cancel a queued or running async solve task",
                    "parameters": [
                        { "name": "task_id", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": { "description": "Task marked cancelled", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TaskRecord" } } } },
                        "400": { "description": "Task already finished" },
                        "404": { "description": "Unknown task id" }
                    }
                }
            },
            "/v1/biz_advice_report": {
                "get": {
                    "summary": "Generate cleaned business advice report from async task output",
                    "parameters": [
                        { "name": "task_id", "in": "query", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": { "description": "Cleaned business advice report", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/BizAdviceReportResponse" } } } }
                    }
                }
            },
            "/v1/project/claude/{ds_id}": {
                "get": {
                    "summary": "Get project CLAUDE.md for ds (from ds_home/home)",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "Current CLAUDE.md", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ProjectClaudeResponse" } } } }
                    }
                },
                "post": {
                    "summary": "Update project CLAUDE.md for ds and sync to git",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateProjectClaudeRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Updated CLAUDE.md", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ProjectClaudeResponse" } } } }
                    }
                }
            },
            "/v1/project/skills/{ds_id}": {
                "post": {
                    "summary": "Create or update a skill for ds and sync to git",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpsertProjectSkillRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Skill upserted", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ProjectSkillResponse" } } } }
                    }
                }
            },
            "/v1/project/prompt/{ds_id}/effective": {
                "get": {
                    "summary": "Get effective system prompt for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "Effective system prompt", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/EffectivePromptResponse" } } } }
                    }
                },
                "post": {
                    "summary": "Reload and get effective system prompt for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "Effective system prompt", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/EffectivePromptResponse" } } } }
                    }
                }
            },
            "/v1/mcp/inject": {
                "post": {
                    "summary": "Inject MCP servers",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/InjectMcpRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Injection result", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/McpResponse" } } } }
                    }
                }
            },
            "/v1/mcp/injected/{ds_id}": {
                "get": {
                    "summary": "Get MCP servers for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } },
                        { "name": "probe_timeout_seconds", "in": "query", "required": false, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "MCP status", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/McpResponse" } } } }
                    }
                },
                "delete": {
                    "summary": "Delete MCP servers for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } },
                        { "name": "server_names", "in": "query", "required": false, "schema": { "type": "string" } },
                        { "name": "probe_timeout_seconds", "in": "query", "required": false, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "Delete result", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/McpResponse" } } } }
                    }
                }
            }
        }
    }))
}

fn projects_git_effective_clone_url(url: &str, token: Option<&str>) -> String {
    let base = url.trim();
    if let Some(t) = token.filter(|s| !s.trim().is_empty()) {
        if let Some(rest) = base.strip_prefix("https://") {
            if !rest.contains('@') {
                return format!("https://x-access-token:{t}@{rest}");
            }
        }
    }
    base.to_string()
}

async fn sync_projects_git_remote(cfg: &GatewayConfig, repo_dir: &Path) -> Result<(), ApiError> {
    let git_dir = repo_dir.join(".git");
    if !fs::metadata(&git_dir).await.is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    let url =
        projects_git_effective_clone_url(&cfg.projects_git_url, cfg.projects_git_token.as_deref());
    run_git(repo_dir, &["remote", "set-url", "origin", &url])
        .await
        .map(|_| ())
}

fn ds_work_dir(work_root: &Path, ds_id: i64) -> PathBuf {
    work_root.join(format!("ds_{ds_id}"))
}

fn projects_repo_dir(work_root: &Path) -> PathBuf {
    work_root.join(".claw-code-projects")
}

fn project_claude_paths(work_dir: &Path) -> (PathBuf, PathBuf) {
    (work_dir.join("home/CLAUDE.md"), work_dir.join("CLAUDE.md"))
}

fn normalize_rel_for_git(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `CLAW_PROJECTS_GIT_AUTHOR` is typically `Name <email>`; used for git author/committer env. kejiqing
fn parse_projects_git_author(author: &str) -> (String, String) {
    let s = author.trim();
    if let (Some(i), Some(j)) = (s.find('<'), s.rfind('>')) {
        if i < j {
            let name = s[..i].trim();
            let email = s[i + 1..j].trim();
            if !email.is_empty() {
                let name_owned = if name.is_empty() {
                    "claw-gateway".to_string()
                } else {
                    name.to_string()
                };
                return (name_owned, email.to_string());
            }
        }
    }
    (s.to_string(), "noreply@claw.local".to_string())
}

fn validate_skill_name(skill_name: &str) -> Result<(), ApiError> {
    if skill_name.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "skillName cannot be empty",
        ));
    }
    if skill_name
        .chars()
        .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "skillName only allows [a-zA-Z0-9._-]",
        ));
    }
    Ok(())
}

async fn copy_tree(src_root: &Path, dst_root: &Path) -> Result<(), ApiError> {
    if !fs::metadata(src_root).await.is_ok_and(|m| m.is_dir()) {
        return Ok(());
    }
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src_root.to_path_buf(), dst_root.to_path_buf())];
    while let Some((src_dir, dst_dir)) = stack.pop() {
        fs::create_dir_all(&dst_dir).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create dir during sync failed: {e}"),
            )
        })?;
        let mut entries = fs::read_dir(&src_dir).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read dir during sync failed: {e}"),
            )
        })?;
        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("iterate dir during sync failed: {e}"),
            )
        })? {
            let entry_path = entry.path();
            let dst_path = dst_dir.join(entry.file_name());
            let file_type = entry.file_type().await.map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("read file type during sync failed: {e}"),
                )
            })?;
            if file_type.is_dir() {
                stack.push((entry_path, dst_path));
            } else if file_type.is_file() {
                if let Some(parent) = dst_path.parent() {
                    fs::create_dir_all(parent).await.map_err(|e| {
                        ApiError::new(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("create parent dir during sync failed: {e}"),
                        )
                    })?;
                }
                fs::copy(&entry_path, &dst_path).await.map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("copy file during sync failed: {e}"),
                    )
                })?;
            }
        }
    }
    Ok(())
}

/// In-process solve uses `session_home` as cwd: symlink `home/skills` and root `CLAUDE.md` from
/// `ds_*` (no per-session copy). Pool solve uses read-only bind mounts instead
/// (`DockerPoolManager::run_worker_container`). Author: kejiqing
async fn prepare_inprocess_session_read_through_from_ds(
    ds_base: &Path,
    session_home: &Path,
) -> Result<(), ApiError> {
    #[cfg(unix)]
    {
        let src_dir = ds_base.join("home/skills");
        let link_path = session_home.join("home/skills");
        let home_parent = session_home.join("home");
        fs::create_dir_all(&home_parent).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create session home/ for skills symlink failed: {e}"),
            )
        })?;
        if let Ok(meta) = fs::symlink_metadata(&link_path).await {
            if meta.is_symlink() {
                fs::remove_file(&link_path).await.map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("remove stale skills symlink failed: {e}"),
                    )
                })?;
            } else if meta.is_dir() {
                fs::remove_dir_all(&link_path).await.map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("remove stale skills dir failed: {e}"),
                    )
                })?;
            } else {
                fs::remove_file(&link_path).await.map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("remove stale skills path failed: {e}"),
                    )
                })?;
            }
        }
        if fs::metadata(&src_dir).await.is_ok_and(|m| m.is_dir()) {
            fs::symlink("../../home/skills", &link_path)
                .await
                .map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("symlink session home/skills failed: {e}"),
                    )
                })?;
        }

        let claude_src = ds_base.join("CLAUDE.md");
        let claude_link = session_home.join("CLAUDE.md");
        if let Ok(meta) = fs::symlink_metadata(&claude_link).await {
            if meta.is_symlink() {
                fs::remove_file(&claude_link).await.map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("remove stale CLAUDE.md symlink failed: {e}"),
                    )
                })?;
            } else if meta.is_file() {
                fs::remove_file(&claude_link).await.map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("remove stale session CLAUDE.md failed: {e}"),
                    )
                })?;
            }
        }
        if fs::metadata(&claude_src).await.is_ok_and(|m| m.is_file()) {
            fs::symlink("../../CLAUDE.md", &claude_link)
                .await
                .map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("symlink session CLAUDE.md failed: {e}"),
                    )
                })?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let src = ds_base.join("home/skills");
        let dst = session_home.join("home/skills");
        if fs::metadata(&dst).await.is_ok() {
            let _ = fs::remove_dir_all(&dst).await;
            let _ = fs::remove_file(&dst).await;
        }
        if fs::metadata(&src).await.is_ok_and(|m| m.is_dir()) {
            copy_tree(&src, &dst).await?;
        }
        let claude_src = ds_base.join("CLAUDE.md");
        let claude_dst = session_home.join("CLAUDE.md");
        if fs::metadata(&claude_src).await.is_ok_and(|m| m.is_file()) {
            let _ = fs::remove_file(&claude_dst).await;
            fs::copy(&claude_src, &claude_dst).await.map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("copy session CLAUDE.md failed: {e}"),
                )
            })?;
        }
        Ok(())
    }
}

async fn run_git(cwd: &Path, args: &[&str]) -> Result<String, ApiError> {
    run_git_env(cwd, &[], args).await
}

async fn run_git_env(
    cwd: &Path,
    env_pairs: &[(&str, &str)],
    args: &[&str],
) -> Result<String, ApiError> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }
    let output = cmd
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("git command failed to start: {e}"),
            )
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("git {} failed: {}", args.join(" "), detail),
        ));
    }
    Ok(stdout)
}

/// Retries for `push` after `pull --rebase` when other hosts race on the same branch. kejiqing
const PROJECTS_GIT_PUSH_MAX_ATTEMPTS: u32 = 20;

fn projects_git_message_suggests_push_retry(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("non-fast-forward")
        || m.contains("failed to push")
        || m.contains("! [remote rejected]")
        || m.contains("updates were rejected")
        || m.contains("stale info")
}

async fn projects_git_rebase_in_progress(repo_dir: &Path) -> bool {
    fs::metadata(repo_dir.join(".git/rebase-merge"))
        .await
        .is_ok_and(|m| m.is_dir())
        || fs::metadata(repo_dir.join(".git/rebase-apply"))
            .await
            .is_ok_and(|m| m.is_dir())
}

async fn projects_git_abort_rebase_best_effort(repo_dir: &Path) {
    if projects_git_rebase_in_progress(repo_dir).await {
        let _ = run_git(repo_dir, &["rebase", "--abort"]).await;
    }
}

/// When `pull --rebase` stopped only on `rel_git_path`, take workspace copy and continue. kejiqing
async fn projects_git_try_resolve_rebase_with_workspace(
    repo_dir: &Path,
    projects_git_author: &str,
    src: &Path,
    dst: &Path,
    rel_git_path: &str,
) -> Result<bool, ApiError> {
    if !projects_git_rebase_in_progress(repo_dir).await {
        return Ok(false);
    }
    let unmerged = match run_git(repo_dir, &["diff", "--name-only", "--diff-filter=U"]).await {
        Ok(s) => s,
        Err(_) => return Ok(false),
    };
    let paths: Vec<&str> = unmerged
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if paths.len() == 1 && paths[0] == rel_git_path {
        fs::copy(src, dst).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("re-resolve conflict file from workspace failed: {e}"),
            )
        })?;
        run_git(repo_dir, &["add", rel_git_path]).await?;
        let (git_name, git_email) = parse_projects_git_author(projects_git_author);
        run_git_env(
            repo_dir,
            &[
                ("GIT_AUTHOR_NAME", git_name.as_str()),
                ("GIT_AUTHOR_EMAIL", git_email.as_str()),
                ("GIT_COMMITTER_NAME", git_name.as_str()),
                ("GIT_COMMITTER_EMAIL", git_email.as_str()),
                ("GIT_EDITOR", "true"),
            ],
            &["rebase", "--continue"],
        )
        .await?;
        return Ok(true);
    }
    Ok(false)
}

async fn ensure_projects_repo_ready(
    work_root: &Path,
    cfg: &GatewayConfig,
) -> Result<PathBuf, ApiError> {
    let repo_dir = projects_repo_dir(work_root);
    if fs::metadata(&repo_dir).await.is_ok_and(|m| m.is_dir()) {
        sync_projects_git_remote(cfg, &repo_dir).await?;
        return Ok(repo_dir);
    }
    fs::create_dir_all(work_root).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create work root failed: {e}"),
        )
    })?;
    let clone_url =
        projects_git_effective_clone_url(&cfg.projects_git_url, cfg.projects_git_token.as_deref());
    run_git(
        work_root,
        &[
            "clone",
            "--branch",
            cfg.projects_git_branch.as_str(),
            &clone_url,
            ".claw-code-projects",
        ],
    )
    .await?;
    Ok(repo_dir)
}

async fn pull_projects_repo(repo_dir: &Path, cfg: &GatewayConfig) -> Result<(), ApiError> {
    sync_projects_git_remote(cfg, repo_dir).await?;
    run_git(repo_dir, &["checkout", cfg.projects_git_branch.as_str()]).await?;
    run_git(
        repo_dir,
        &[
            "pull",
            "--ff-only",
            "origin",
            cfg.projects_git_branch.as_str(),
        ],
    )
    .await?;
    Ok(())
}

async fn sync_ds_home_from_repo(
    repo_dir: &Path,
    work_dir: &Path,
    ds_id: i64,
) -> Result<(), ApiError> {
    let ds_repo_home = repo_dir.join(format!("ds_{ds_id}/home"));
    let ds_work_home = work_dir.join("home");
    if fs::metadata(&ds_work_home).await.is_ok_and(|m| m.is_dir()) {
        fs::remove_dir_all(&ds_work_home).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cleanup stale ds home failed: {e}"),
            )
        })?;
    }
    if fs::metadata(&ds_repo_home).await.is_ok_and(|m| m.is_dir()) {
        copy_tree(&ds_repo_home, &ds_work_home).await?;
    } else {
        fs::create_dir_all(&ds_work_home).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create empty ds home failed: {e}"),
            )
        })?;
    }
    let (home_claude, root_claude) = project_claude_paths(work_dir);
    if fs::metadata(&home_claude).await.is_ok_and(|m| m.is_file()) {
        fs::copy(&home_claude, &root_claude).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("mirror home CLAUDE.md to root failed: {e}"),
            )
        })?;
    }
    Ok(())
}

/// Clone-or-open mirror repo and `git pull --ff-only` (no local `ds_*` writes). Caller must hold
/// [`AppState::projects_git_mirror_lock`] for the duration if other tasks may touch the mirror. kejiqing
async fn projects_git_mirror_pull_impl(
    work_root: &Path,
    cfg: &GatewayConfig,
) -> Result<PathBuf, ApiError> {
    let repo_dir = ensure_projects_repo_ready(work_root, cfg).await?;
    pull_projects_repo(&repo_dir, cfg).await?;
    Ok(repo_dir)
}

/// Copy one file from `ds_<id>/` workspace into the mirror, then commit + push. Caller must hold
/// `projects_git_mirror_lock` (same critical section as pull) and `ds_lock` for `ds_id`.
///
/// Other hosts may push the same branch: after commit we `pull --rebase` and retry `push` with
/// backoff; if rebase stops only on this path, workspace content wins. kejiqing
async fn projects_git_mirror_copy_commit_push_impl(
    cfg: &GatewayConfig,
    work_root: &Path,
    repo_dir: &Path,
    ds_id: i64,
    rel_path_under_ds: &Path,
    commit_message: &str,
) -> Result<GitSyncResponse, ApiError> {
    let work_dir = ds_work_dir(work_root, ds_id);
    let src = work_dir.join(rel_path_under_ds);
    let ds_root_in_repo = repo_dir.join(format!("ds_{ds_id}"));
    let dst = ds_root_in_repo.join(rel_path_under_ds);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create repo parent dir failed: {e}"),
            )
        })?;
    }
    fs::copy(&src, &dst).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("copy file into git repo failed: {e}"),
        )
    })?;
    let rel_git_path = format!("ds_{ds_id}/{}", normalize_rel_for_git(rel_path_under_ds));
    run_git(repo_dir, &["add", &rel_git_path]).await?;
    let dirty = run_git(repo_dir, &["status", "--porcelain", "--", &rel_git_path]).await?;
    let mut pushed = false;
    if !dirty.trim().is_empty() {
        sync_projects_git_remote(cfg, repo_dir).await?;
        let (git_name, git_email) = parse_projects_git_author(cfg.projects_git_author.as_str());
        run_git_env(
            repo_dir,
            &[
                ("GIT_AUTHOR_NAME", git_name.as_str()),
                ("GIT_AUTHOR_EMAIL", git_email.as_str()),
                ("GIT_COMMITTER_NAME", git_name.as_str()),
                ("GIT_COMMITTER_EMAIL", git_email.as_str()),
            ],
            &[
                "commit",
                "--author",
                cfg.projects_git_author.as_str(),
                "-m",
                commit_message,
            ],
        )
        .await?;

        let branch = cfg.projects_git_branch.as_str();
        for attempt in 0..PROJECTS_GIT_PUSH_MAX_ATTEMPTS {
            sync_projects_git_remote(cfg, repo_dir).await?;

            match run_git(repo_dir, &["pull", "--rebase", "origin", branch]).await {
                Ok(_) => {}
                Err(e) => {
                    let detail = e.detail();
                    if projects_git_rebase_in_progress(repo_dir).await {
                        if projects_git_try_resolve_rebase_with_workspace(
                            repo_dir,
                            cfg.projects_git_author.as_str(),
                            &src,
                            &dst,
                            &rel_git_path,
                        )
                        .await?
                        {
                            continue;
                        }
                        projects_git_abort_rebase_best_effort(repo_dir).await;
                        return Err(ApiError::new(
                            StatusCode::CONFLICT,
                            format!(
                                "projects git rebase conflict (multiple writers or overlapping paths): {detail}"
                            ),
                        ));
                    }
                    return Err(e);
                }
            }

            match run_git(repo_dir, &["push", "origin", branch]).await {
                Ok(_) => {
                    pushed = true;
                    break;
                }
                Err(e) => {
                    let detail = e.detail();
                    if projects_git_message_suggests_push_retry(detail)
                        && attempt + 1 < PROJECTS_GIT_PUSH_MAX_ATTEMPTS
                    {
                        let ms = 40_u64.saturating_mul(1_u64 << attempt.min(8));
                        tokio::time::sleep(Duration::from_millis(ms)).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        if !pushed {
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "projects git push exhausted retries (remote busy or concurrent writers)",
            ));
        }
    }
    let commit_id = run_git(repo_dir, &["rev-parse", "HEAD"]).await?;
    Ok(GitSyncResponse {
        repo: cfg.projects_git_url.clone(),
        branch: cfg.projects_git_branch.clone(),
        commit_id,
        pushed,
    })
}

async fn projects_git_ds_home_poll_loop(state: AppState, interval_secs: u64) {
    let mut ticker = interval(Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        match tick_projects_git_ds_home_poll(&state).await {
            Ok(()) => {}
            Err(e) => {
                warn!(
                    target: "claw_gateway_orchestration",
                    component = "projects_git_poll",
                    phase = "tick_failed",
                    status = %e.status,
                    error = %e.detail(),
                    "periodic project mirror / ds home sync failed"
                );
            }
        }
    }
}

async fn tick_projects_git_ds_home_poll(state: &AppState) -> Result<(), ApiError> {
    let repo_dir = {
        let _mirror = state.projects_git_mirror_lock.lock().await;
        projects_git_mirror_pull_impl(&state.cfg.work_root, state.cfg.as_ref()).await?
    };
    let ids = list_ds_ids_under_work_root(&state.cfg.work_root).await?;
    for ds_id in ids {
        let lock = get_ds_lock(state, ds_id).await;
        let Ok(_guard) = lock.try_lock() else {
            continue;
        };
        let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
        if !fs::metadata(work_dir.join(".claw"))
            .await
            .is_ok_and(|m| m.is_dir())
        {
            continue;
        }
        sync_ds_home_from_repo(&repo_dir, &work_dir, ds_id).await?;
    }
    Ok(())
}

async fn list_ds_ids_under_work_root(work_root: &Path) -> Result<Vec<i64>, ApiError> {
    let mut out = Vec::new();
    let mut rd = fs::read_dir(work_root).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("list work_root for ds poll failed: {e}"),
        )
    })?;
    while let Some(ent) = rd.next_entry().await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read work_root entry failed: {e}"),
        )
    })? {
        let name = ent.file_name().to_string_lossy().to_string();
        let Some(rest) = name.strip_prefix("ds_") else {
            continue;
        };
        if let Ok(id) = rest.parse::<i64>() {
            if id >= 1 {
                out.push(id);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

async fn healthz(State(state): State<AppState>) -> Json<Value> {
    let isolation = state.cfg.solve_isolation.as_str();
    Json(json!({
        "ok": true,
        "clawBin": state.cfg.claw_bin,
        "workRoot": state.cfg.work_root.display().to_string(),
        "registryPath": state.cfg.ds_registry_path.display().to_string(),
        "defaultTimeoutSeconds": state.cfg.default_timeout_seconds,
        "defaultMaxIterations": state.cfg.default_max_iterations,
        "defaultHttpMcpName": state.cfg.default_http_mcp_name,
        "defaultHttpMcpUrl": state.cfg.default_http_mcp_url,
        "defaultHttpMcpTransport": state.cfg.default_http_mcp_transport,
        "allowedTools": state.cfg.allowed_tools,
        "solveIsolation": isolation,
        "containerPool": state.docker_pool.is_some(),
        "poolRpcRemote": state.cfg.pool_rpc_remote,
        "poolRpcTcp": state.cfg.pool_rpc_tcp,
        "poolRpcUnixSocket": state.cfg.pool_rpc_unix_socket,
        "poolRpcHostWorkRoot": state.cfg.pool_rpc_host_work_root.as_ref().map(|p| p.display().to_string()),
        "sessionDbPath": state.session_db.path().display().to_string(),
        "projectsGitUrl": state.cfg.projects_git_url.clone(),
        "projectsGitBranch": state.cfg.projects_git_branch.clone(),
        "projectsGitDsHomePollIntervalSecs": state.cfg.projects_git_ds_home_poll_interval_secs,
    }))
}

async fn solve(
    State(state): State<AppState>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Extension(id_kind): Extension<session_merge::HttpRequestIdKind>,
    Json(req): Json<SolveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let body_sid = session_merge::trim_session_id(req.session_id.as_deref());
    let effective =
        session_merge::merge_effective_session_id(body_sid, &http_request_id.0, id_kind)
            .map_err(session_routing_error)?;
    info!(
        request_id = %effective,
        ds_id = req.ds_id,
        endpoint = "/v1/solve",
        phase = "accepted",
        "gateway_solve"
    );
    let result = run_solve_request(
        state,
        req,
        RunSolveContext {
            request_id: effective.clone(),
            task_id: None,
            skip_session_db: false,
        },
    )
    .await?;
    let claw = HeaderValue::from_str(&effective).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid characters in session id for response header",
        )
    })?;
    let xrid = header::HeaderName::from_static("x-request-id");
    let csid = header::HeaderName::from_static("claw-session-id");
    Ok((
        AppendHeaders([(xrid, claw.clone()), (csid, claw)]),
        Json(result),
    ))
}

async fn init_workspace(
    State(state): State<AppState>,
    Json(req): Json<InitRequest>,
) -> Result<Json<InitResponse>, ApiError> {
    if req.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let work_dir = ds_work_dir(&state.cfg.work_root, req.ds_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    {
        let _mirror = state.projects_git_mirror_lock.lock().await;
        let repo_dir =
            projects_git_mirror_pull_impl(&state.cfg.work_root, state.cfg.as_ref()).await?;
        sync_ds_home_from_repo(&repo_dir, &work_dir, req.ds_id).await?;
    }
    {
        let lock = get_ds_lock(&state, req.ds_id).await;
        let _guard = lock.lock().await;
        ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
        let settings = build_settings(&state, req.ds_id).await;
        let settings_content = serde_json::to_vec_pretty(&settings).map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize settings failed: {e}"),
            )
        })?;
        fs::write(work_dir.join(".claw/settings.json"), settings_content)
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write settings failed: {e}"),
                )
            })?;
        let claude_md_path = work_dir.join("CLAUDE.md");
        match fs::metadata(&claude_md_path).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::write(&claude_md_path, "").await.map_err(|e| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("write CLAUDE.md failed: {e}"),
                    )
                })?;
            }
            Err(error) => {
                return Err(ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("stat CLAUDE.md failed: {error}"),
                ));
            }
        }
    }
    Ok(Json(InitResponse {
        ds_id: req.ds_id,
        work_dir: work_dir.display().to_string(),
        initialized: true,
    }))
}

async fn get_project_claude_md(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<ProjectClaudeResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let (home_claude_md_path, root_claude_md_path) = project_claude_paths(&work_dir);
    let content = fs::read_to_string(&home_claude_md_path).await;
    let (exists, content) = match content {
        Ok(text) => (true, text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::read_to_string(&root_claude_md_path).await {
                Ok(text) => (true, text),
                Err(root_err) if root_err.kind() == std::io::ErrorKind::NotFound => {
                    (false, String::new())
                }
                Err(root_err) => {
                    return Err(ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("read CLAUDE.md failed: {root_err}"),
                    ));
                }
            }
        }
        Err(error) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read CLAUDE.md failed: {error}"),
            ));
        }
    };
    Ok(Json(ProjectClaudeResponse {
        ds_id,
        work_dir: work_dir.display().to_string(),
        path: home_claude_md_path.display().to_string(),
        exists,
        content,
    }))
}

async fn update_project_claude_md(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Json(req): Json<UpdateProjectClaudeRequest>,
) -> Result<Json<ProjectClaudeResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let git_sync = {
        let _mirror = state.projects_git_mirror_lock.lock().await;
        let lock = get_ds_lock(&state, ds_id).await;
        let _guard = lock.lock().await;
        ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
        let repo_dir =
            projects_git_mirror_pull_impl(&state.cfg.work_root, state.cfg.as_ref()).await?;
        let (home_claude_md_path, root_claude_md_path) = project_claude_paths(&work_dir);
        if let Some(parent) = home_claude_md_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("create home dir failed: {e}"),
                )
            })?;
        }
        fs::write(&home_claude_md_path, req.content.as_bytes())
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write home/CLAUDE.md failed: {e}"),
                )
            })?;
        fs::write(&root_claude_md_path, req.content.as_bytes())
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write CLAUDE.md failed: {e}"),
                )
            })?;
        projects_git_mirror_copy_commit_push_impl(
            state.cfg.as_ref(),
            &state.cfg.work_root,
            &repo_dir,
            ds_id,
            Path::new("home/CLAUDE.md"),
            &format!("update ds_{ds_id} CLAUDE.md"),
        )
        .await?
    };
    info!(
        target: "claw_gateway_orchestration",
        component = "project_claude",
        ds_id,
        branch = %git_sync.branch,
        commit_id = %git_sync.commit_id,
        pushed = git_sync.pushed,
        "project CLAUDE.md git synced"
    );
    let claude_md_path = work_dir.join("home/CLAUDE.md");
    Ok(Json(ProjectClaudeResponse {
        ds_id,
        work_dir: work_dir.display().to_string(),
        path: claude_md_path.display().to_string(),
        exists: true,
        content: req.content,
    }))
}

async fn upsert_project_skill(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Json(req): Json<UpsertProjectSkillRequest>,
) -> Result<Json<ProjectSkillResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let skill_name = req.skill_name.trim().to_string();
    validate_skill_name(&skill_name)?;
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    let skill_rel = PathBuf::from("home")
        .join("skills")
        .join(&skill_name)
        .join("SKILL.md");
    let skill_path = work_dir.join(&skill_rel);
    let existed = fs::metadata(&skill_path).await.is_ok_and(|m| m.is_file());
    let git_sync = {
        let _mirror = state.projects_git_mirror_lock.lock().await;
        let lock = get_ds_lock(&state, ds_id).await;
        let _guard = lock.lock().await;
        ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
        let repo_dir =
            projects_git_mirror_pull_impl(&state.cfg.work_root, state.cfg.as_ref()).await?;
        if let Some(parent) = skill_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("create skill dir failed: {e}"),
                )
            })?;
        }
        fs::write(&skill_path, req.skill_content.as_bytes())
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write SKILL.md failed: {e}"),
                )
            })?;
        projects_git_mirror_copy_commit_push_impl(
            state.cfg.as_ref(),
            &state.cfg.work_root,
            &repo_dir,
            ds_id,
            &skill_rel,
            &format!("upsert ds_{ds_id} skill {skill_name}"),
        )
        .await?
    };
    Ok(Json(ProjectSkillResponse {
        ds_id,
        skill_name,
        skill_path: skill_path.display().to_string(),
        created: !existed,
        updated: existed,
        bytes_written: req.skill_content.len(),
        work_dir: work_dir.display().to_string(),
        git_sync,
    }))
}

async fn get_effective_prompt(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<EffectivePromptResponse>, ApiError> {
    build_effective_prompt_response(&state, ds_id)
        .await
        .map(Json)
}

async fn post_effective_prompt(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<EffectivePromptResponse>, ApiError> {
    build_effective_prompt_response(&state, ds_id)
        .await
        .map(Json)
}

async fn build_effective_prompt_response(
    state: &AppState,
    ds_id: i64,
) -> Result<EffectivePromptResponse, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let work_dir = state.cfg.work_root.join(format!("ds_{ds_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let sections = load_system_prompt(
        work_dir.to_path_buf(),
        default_system_date(),
        std::env::consts::OS,
        "unknown",
        None,
    )
    .map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("load system prompt failed: {e}"),
        )
    })?;
    let message = sections.join("\n\n");
    Ok(EffectivePromptResponse {
        ds_id,
        work_dir: work_dir.display().to_string(),
        sections,
        message,
    })
}

async fn solve_async(
    State(state): State<AppState>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Extension(id_kind): Extension<session_merge::HttpRequestIdKind>,
    Json(req): Json<SolveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let body_sid = session_merge::trim_session_id(req.session_id.as_deref());
    let effective =
        session_merge::merge_effective_session_id(body_sid, &http_request_id.0, id_kind)
            .map_err(session_routing_error)?;
    if session_merge::trim_session_id(req.session_id.as_deref()).is_some() {
        let row = state
            .session_db
            .get_session_home_rel(&effective, req.ds_id)
            .await
            .map_err(|e| session_db_err(&e))?;
        if row.is_none() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "unknown sessionId (no session history for this dsId)",
            ));
        }
    }
    let task_id = effective.clone();
    info!(
        request_id = %effective,
        task_id = %task_id,
        ds_id = req.ds_id,
        endpoint = "/v1/solve_async",
        phase = "queued",
        "gateway_solve_async"
    );
    {
        let mut tasks = state.tasks.lock().await;
        if let Some(inner) = tasks.get(&task_id) {
            if inner.record.status == "queued" || inner.record.status == "running" {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "session has active async task",
                ));
            }
        }
        tasks.insert(
            task_id.clone(),
            TaskInner {
                record: TaskRecord {
                    task_id: task_id.clone(),
                    session_id: effective.clone(),
                    request_id: effective.clone(),
                    status: "queued".to_string(),
                    created_at_ms: now_ms(),
                    started_at_ms: None,
                    finished_at_ms: None,
                    result: None,
                    error: None,
                },
                cancel: None,
            },
        );
    }
    let state_clone = state.clone();
    let task_id_for_worker = task_id.clone();
    let rid = effective.clone();
    let join = tokio::spawn(async move {
        {
            let mut tasks = state_clone.tasks.lock().await;
            if let Some(inner) = tasks.get_mut(&task_id_for_worker) {
                if inner.record.status == "cancelled" {
                    inner.cancel = None;
                    return;
                }
                inner.record.status = "running".to_string();
                inner.record.started_at_ms = Some(now_ms());
            }
        }
        info!(
            request_id = %rid,
            task_id = %task_id_for_worker,
            phase = "running",
            "gateway_solve_async"
        );
        let result = run_solve_request(
            state_clone.clone(),
            req,
            RunSolveContext {
                request_id: rid.clone(),
                task_id: Some(task_id_for_worker.clone()),
                skip_session_db: false,
            },
        )
        .await;
        let mut tasks = state_clone.tasks.lock().await;
        if let Some(inner) = tasks.get_mut(&task_id_for_worker) {
            inner.cancel = None;
            if inner.record.status == "cancelled" {
                return;
            }
            inner.record.finished_at_ms = Some(now_ms());
            match result {
                Ok(v) => {
                    let duration_ms = v.duration_ms;
                    inner.record.status = "succeeded".to_string();
                    inner.record.result = Some(v);
                    info!(
                        request_id = %rid,
                        task_id = %task_id_for_worker,
                        phase = "succeeded",
                        duration_ms,
                        "gateway_solve_async"
                    );
                }
                Err(e) => {
                    inner.record.status = "failed".to_string();
                    inner.record.error =
                        Some(json!({"status_code": e.status.as_u16(), "detail": e.message}));
                    warn!(
                        request_id = %rid,
                        task_id = %task_id_for_worker,
                        phase = "failed",
                        status_code = e.status.as_u16(),
                        error = %e.message,
                        "gateway_solve_async"
                    );
                }
            }
        }
    });
    let cancel = join.abort_handle();
    {
        let mut tasks = state.tasks.lock().await;
        if let Some(inner) = tasks.get_mut(&task_id) {
            inner.cancel = Some(cancel);
        }
    }
    let claw = HeaderValue::from_str(&effective).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid characters in session id for response header",
        )
    })?;
    let xrid = header::HeaderName::from_static("x-request-id");
    let csid = header::HeaderName::from_static("claw-session-id");
    Ok((
        AppendHeaders([(xrid, claw.clone()), (csid, claw)]),
        Json(SolveAsyncResponse {
            task_id: task_id.clone(),
            session_id: effective.clone(),
            request_id: effective.clone(),
            status: "queued".to_string(),
            poll_url: format!("/v1/tasks/{task_id}"),
        }),
    ))
}

async fn get_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<TaskRecord>, ApiError> {
    let tasks = state.tasks.lock().await;
    let task = tasks
        .get(&task_id)
        .map(|inner| inner.record.clone())
        .ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, format!("task not found: {task_id}"))
        })?;
    info!(
        request_id = %http_request_id.0,
        task_id = %task_id,
        task_request_id = %task.request_id,
        task_status = %task.status,
        endpoint = "/v1/tasks/{task_id}",
        phase = "poll",
        "gateway_task"
    );
    Ok(Json(task))
}

async fn cancel_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<TaskRecord>, ApiError> {
    let cancel = {
        let mut tasks = state.tasks.lock().await;
        let inner = tasks.get_mut(&task_id).ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, format!("task not found: {task_id}"))
        })?;
        match inner.record.status.as_str() {
            "succeeded" | "failed" | "cancelled" => {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "task {} is already finished (status: {})",
                        task_id, inner.record.status
                    ),
                ));
            }
            _ => {}
        }
        let h = inner.cancel.take();
        inner.record.status = "cancelled".to_string();
        inner.record.finished_at_ms = Some(now_ms());
        inner.record.result = None;
        inner.record.error = Some(json!({"detail": "cancelled by client"}));
        h
    };
    // Stop the container worker before aborting the host task: `kill_on_drop` then tears down
    // the `docker exec` client, and in-flight stderr can still flush while the container exits.
    if let Some((pool, idx)) = state.docker_slots.lock().await.remove(&task_id) {
        let _ = pool.force_kill_slot(idx).await;
    }
    if let Some(h) = cancel {
        h.abort();
    }
    info!(
        request_id = %http_request_id.0,
        task_id = %task_id,
        endpoint = "/v1/tasks/{task_id}/cancel",
        phase = "cancel",
        "gateway_task"
    );
    let tasks = state.tasks.lock().await;
    let record = tasks
        .get(&task_id)
        .map(|inner| inner.record.clone())
        .ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, format!("task not found: {task_id}"))
        })?;
    Ok(Json(record))
}

async fn get_biz_advice_report(
    State(state): State<AppState>,
    Query(query): Query<BizAdviceReportQuery>,
) -> Result<Json<BizAdviceReportResponse>, ApiError> {
    let task = {
        let tasks = state.tasks.lock().await;
        tasks
            .get(&query.task_id)
            .map(|inner| inner.record.clone())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::NOT_FOUND,
                    format!("task not found: {}", query.task_id),
                )
            })?
    };
    let source_status = task.status.clone();
    if source_status != "succeeded" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "task {} is not succeeded yet (status: {})",
                query.task_id, source_status
            ),
        ));
    }
    let source_result = task.result.ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "task {} has no result yet (status: {})",
                query.task_id, source_status
            ),
        )
    })?;
    let raw_json = source_result.output_json.as_ref().map_or_else(
        || "null".to_string(),
        |v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
    );
    let prompt = format!(
        "你是资深业务分析顾问。下面给你一段原始输出，其中包含中间过程、思考草稿或噪声信息。\n\
请只输出“最终干净报告”，要求：\n\
1) 不要输出任何中间过程、思考轨迹、工具调用痕迹。\n\
2) 结构清晰，使用简洁中文。\n\
3) 保留关键结论、依据与可执行建议。\n\
4) 如果信息不足，明确写出“信息不足”并给出最小补充数据清单。\n\
5) 不要添加与原文无关的事实。\n\n\
【原始文本输出】\n{}\n\n\
【原始 JSON 输出】\n{}",
        source_result.output_text, raw_json
    );
    let report = run_solve_request(
        state,
        SolveRequest {
            ds_id: source_result.ds_id,
            user_prompt: prompt,
            session_id: None,
            model: None,
            timeout_seconds: None,
            extra_session: None,
            allowed_tools: None,
        },
        RunSolveContext {
            request_id: Uuid::new_v4().simple().to_string(),
            task_id: None,
            skip_session_db: true,
        },
    )
    .await?;
    Ok(Json(BizAdviceReportResponse {
        task_id: query.task_id,
        source_request_id: task.request_id,
        source_ds_id: source_result.ds_id,
        source_status,
        report_text: report.output_text,
        report_json: report.output_json,
    }))
}

async fn run_solve_request(
    state: AppState,
    req: SolveRequest,
    ctx: RunSolveContext,
) -> Result<SolveResponse, ApiError> {
    if req.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    if req.user_prompt.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "userPrompt cannot be empty",
        ));
    }
    if let Some(extra_session) = &req.extra_session {
        if !extra_session.is_object() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "extraSession must be a JSON object when present",
            ));
        }
        if let Ok(serialized) = serde_json::to_vec(extra_session) {
            if serialized.len() > 8 * 1024 {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "extraSession is too large (max 8KB)",
                ));
            }
        }
    }
    let started = Instant::now();
    let timeout_seconds = req
        .timeout_seconds
        .unwrap_or(state.cfg.default_timeout_seconds);
    info!(
        target: "claw_gateway_orchestration",
        component = "solve",
        request_id = %ctx.request_id,
        task_id = ctx.task_id.as_deref().unwrap_or("-"),
        ds_id = req.ds_id,
        phase = "solve_run_start",
        timeout_seconds,
        "gateway_solve accepted; validating and preparing workspace"
    );
    let effective_allowed_tools =
        resolve_effective_allowed_tools(&state.cfg.allowed_tools, req.allowed_tools.as_deref())?;
    validate_ds_exists(req.ds_id, &state.cfg.ds_registry_path).await?;

    let _session_lock_guard: Option<OwnedMutexGuard<()>> = if ctx.skip_session_db {
        None
    } else {
        Some(
            get_session_solve_lock(&state, req.ds_id, &ctx.request_id)
                .await
                .lock_owned()
                .await,
        )
    };

    let ds_base = state.cfg.work_root.join(format!("ds_{}", req.ds_id));
    let explicit_continuation = session_merge::trim_session_id(req.session_id.as_deref()).is_some();

    let (session_home, need_insert_row, purge_mcp_discovery, session_fs_label) =
        if ctx.skip_session_db {
            let session_fs_id = session_merge::sessions_directory_segment(&ctx.request_id);
            let session_home = ds_base.join("sessions").join(&session_fs_id);
            (session_home, false, true, session_fs_id)
        } else {
            let row_opt = state
                .session_db
                .get_session_home_rel(&ctx.request_id, req.ds_id)
                .await
                .map_err(|e| session_db_err(&e))?;
            if let Some(rel) = row_opt {
                session_merge::validate_session_home_rel(&rel).map_err(session_routing_error)?;
                let session_home =
                    session_merge::join_session_home_from_rel(&state.cfg.work_root, &rel);
                let exists = fs::metadata(&session_home).await.is_ok_and(|m| m.is_dir());
                if !exists {
                    return Err(ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "session workspace is missing on disk (database path is stale)",
                    ));
                }
                (session_home, false, false, rel)
            } else if explicit_continuation {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "unknown sessionId (no session history for this dsId)",
                ));
            } else {
                let session_fs_id = session_merge::sessions_directory_segment(&ctx.request_id);
                let session_home = ds_base.join("sessions").join(&session_fs_id);
                (session_home, true, true, session_fs_id)
            }
        };

    let session_home_rel =
        session_merge::session_home_rel_under_work_root(&state.cfg.work_root, &session_home)
            .map_err(session_routing_error)?;

    fs::create_dir_all(session_home.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create session work dir failed: {e}"),
            )
        })?;

    {
        let ds_lock = get_ds_lock(&state, req.ds_id).await;
        let _guard = ds_lock.lock().await;
        fs::create_dir_all(&ds_base).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create ds dir failed: {e}"),
            )
        })?;
        ensure_workspace_initialized(&state.cfg.claw_bin, &ds_base).await?;
        if matches!(state.cfg.solve_isolation, SolveIsolation::InProcess) {
            prepare_inprocess_session_read_through_from_ds(&ds_base, &session_home).await?;
        }
        let settings = build_settings(&state, req.ds_id).await;
        let settings_content = serde_json::to_vec_pretty(&settings).map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize settings failed: {e}"),
            )
        })?;
        fs::write(session_home.join(".claw/settings.json"), &settings_content)
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write settings failed: {e}"),
                )
            })?;
        if purge_mcp_discovery {
            let _ = fs::remove_file(session_home.join(".claw/mcp_discovery_cache.json")).await;
        }
    }

    if need_insert_row {
        state
            .session_db
            .insert_session(&ctx.request_id, req.ds_id, &session_home_rel, now_ms())
            .await
            .map_err(|e| session_db_err(&e))?;
    } else if !ctx.skip_session_db {
        state
            .session_db
            .touch_updated(&ctx.request_id, req.ds_id, now_ms())
            .await
            .map_err(|e| session_db_err(&e))?;
    }

    info!(
        target: "claw_gateway_orchestration",
        component = "solve_prepare",
        phase = "workspace_ready",
        ds_id = req.ds_id,
        request_id = %ctx.request_id,
        task_id = ctx.task_id.as_deref(),
        session_fs_id = %session_fs_label,
        session_home = %session_home.display(),
        solve_isolation = state.cfg.solve_isolation.as_str(),
        timeout_seconds,
        "session .claw/settings.json written; starting solve (in-process or pool)"
    );

    match state.cfg.solve_isolation {
        SolveIsolation::InProcess => {
            let (code, output_text, output_json) = run_runtime_prompt(
                &session_home,
                &state.cfg.work_root,
                &req.user_prompt,
                req.model.as_deref(),
                timeout_seconds,
                &ctx.request_id,
                req.extra_session.clone(),
                effective_allowed_tools,
                state.cfg.default_max_iterations,
            )?;
            let duration_ms = started.elapsed().as_millis() as i64;
            info!(
                target: "claw_gateway_orchestration",
                component = "solve_inprocess",
                request_id = %ctx.request_id,
                task_id = ctx.task_id.as_deref().unwrap_or("-"),
                ds_id = req.ds_id,
                phase = "solve_run_ok",
                duration_ms,
                session_home = %session_home.display(),
                claw_exit_code = code,
                "in-process gateway_solve finished"
            );
            Ok(SolveResponse {
                session_id: ctx.request_id.clone(),
                request_id: ctx.request_id,
                session_home_rel: session_home_rel.clone(),
                ds_id: req.ds_id,
                work_dir: session_home.display().to_string(),
                duration_ms,
                claw_exit_code: code,
                output_text,
                output_json,
            })
        }
        SolveIsolation::DockerPool | SolveIsolation::PodmanPool => {
            let pool = state.docker_pool.clone().ok_or_else(|| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "container pool is not initialized",
                )
            })?;
            solve_pool::run_solve_request_docker(
                state,
                req,
                ctx,
                pool,
                started,
                effective_allowed_tools,
                solve_pool::SolveSessionPaths {
                    session_home,
                    session_home_rel,
                },
            )
            .await
        }
    }
}

async fn inject_mcp(
    State(state): State<AppState>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Json(req): Json<InjectMcpRequest>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    if req.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let replace = req.replace.unwrap_or(false);
    {
        let mut injected = state.injected_mcp.lock().await;
        if replace {
            injected.insert(req.ds_id, req.mcp_servers.clone());
        } else {
            let current = injected.entry(req.ds_id).or_default();
            for (k, v) in req.mcp_servers {
                current.insert(k, v);
            }
        }
    }
    let (report, loaded_names, configured_servers, status, names) =
        apply_settings_and_probe(&state, req.ds_id, 15).await?;
    let loaded = names.iter().all(|name| loaded_names.contains(name)) && status == "ok";
    let missing_servers = names
        .iter()
        .filter(|name| !loaded_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(McpResponse {
        session_id: request_id.clone(),
        request_id,
        ds_id: req.ds_id,
        injected_server_names: names,
        loaded,
        missing_servers,
        configured_servers,
        status,
        mcp_report: report,
    }))
}

async fn get_injected_mcp(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Query(query): Query<ProbeQuery>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    let timeout_seconds = query.probe_timeout_seconds.unwrap_or(15);
    let (report, loaded_names, configured_servers, status, names) =
        apply_settings_and_probe(&state, ds_id, timeout_seconds).await?;
    let loaded = names.iter().all(|name| loaded_names.contains(name)) && status == "ok";
    let missing_servers = names
        .iter()
        .filter(|name| !loaded_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(McpResponse {
        session_id: request_id.clone(),
        request_id,
        ds_id,
        injected_server_names: names,
        loaded,
        missing_servers,
        configured_servers,
        status,
        mcp_report: report,
    }))
}

async fn delete_injected_mcp(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<McpResponse>, ApiError> {
    let request_id = http_request_id.0.clone();
    {
        let mut injected = state.injected_mcp.lock().await;
        if let Some(names) = query.server_names {
            let targets = names
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            let current = injected.entry(ds_id).or_default();
            for name in targets {
                current.remove(&name);
            }
            if current.is_empty() {
                injected.remove(&ds_id);
            }
        } else {
            injected.remove(&ds_id);
        }
    }
    let timeout_seconds = query.probe_timeout_seconds.unwrap_or(15);
    let (report, loaded_names, configured_servers, status, names) =
        apply_settings_and_probe(&state, ds_id, timeout_seconds).await?;
    let loaded = names.iter().all(|name| loaded_names.contains(name)) && status == "ok";
    let missing_servers = names
        .iter()
        .filter(|name| !loaded_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(McpResponse {
        session_id: request_id.clone(),
        request_id,
        ds_id,
        injected_server_names: names,
        loaded,
        missing_servers,
        configured_servers,
        status,
        mcp_report: report,
    }))
}

async fn apply_settings_and_probe(
    state: &AppState,
    ds_id: i64,
    probe_timeout_seconds: u64,
) -> Result<(Value, Vec<String>, i64, String, Vec<String>), ApiError> {
    let work_dir = state.cfg.work_root.join(format!("ds_{ds_id}"));
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create work dir failed: {e}"),
            )
        })?;
    {
        let lock = get_ds_lock(state, ds_id).await;
        let _guard = lock.lock().await;
        ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
        let settings = build_settings(state, ds_id).await;
        let settings_content = serde_json::to_vec_pretty(&settings).map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("serialize settings failed: {e}"),
            )
        })?;
        fs::write(work_dir.join(".claw/settings.json"), settings_content)
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("write settings failed: {e}"),
                )
            })?;
        let _ = fs::remove_file(work_dir.join(".claw/mcp_discovery_cache.json")).await;
    }
    let (report, loaded_names, configured_servers, status) =
        probe_mcp_load(&state.cfg.claw_bin, &work_dir, probe_timeout_seconds).await?;
    let names = {
        let injected = state.injected_mcp.lock().await;
        injected
            .get(&ds_id)
            .map(|v| v.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    Ok((report, loaded_names, configured_servers, status, names))
}

async fn build_settings(state: &AppState, ds_id: i64) -> Value {
    let mut servers = HashMap::<String, Value>::new();
    for (k, v) in &state.cfg.config_mcp_servers {
        servers.insert(k.clone(), v.clone());
    }
    if let (Some(name), Some(url)) = (
        state.cfg.default_http_mcp_name.as_ref(),
        state.cfg.default_http_mcp_url.as_ref(),
    ) {
        servers.insert(
            name.clone(),
            json!({
                "type": state.cfg.default_http_mcp_transport,
                "url": url
            }),
        );
    }
    let injected = state.injected_mcp.lock().await;
    if let Some(extra) = injected.get(&ds_id) {
        for (k, v) in extra {
            servers.insert(k.clone(), v.clone());
        }
    }
    json!({ "mcpServers": servers })
}

async fn ensure_workspace_initialized(_claw_bin: &str, work_dir: &Path) -> Result<(), ApiError> {
    let marker = work_dir.join(".claw/.gateway_init_done");
    if fs::metadata(&marker).await.is_ok() {
        return Ok(());
    }
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("workspace init failed: {e}"),
            )
        })?;
    fs::write(marker, now_ms().to_string()).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write init marker failed: {e}"),
        )
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_runtime_prompt(
    work_dir: &Path,
    work_root: &Path,
    prompt: &str,
    model: Option<&str>,
    timeout_seconds: u64,
    clawcode_session_id: &str,
    extra_session: Option<Value>,
    allowed_tools: Vec<String>,
    max_iterations: usize,
) -> Result<(i32, String, Option<Value>), ApiError> {
    run_gateway_solve_turn(
        work_dir,
        work_root,
        prompt,
        model,
        timeout_seconds,
        clawcode_session_id,
        extra_session,
        allowed_tools,
        max_iterations,
    )
    .map_err(|e| {
        let status = match e.status {
            504 => StatusCode::GATEWAY_TIMEOUT,
            _ => StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        };
        ApiError::new(status, e.message)
    })
}

async fn probe_mcp_load(
    claw_bin: &str,
    work_dir: &Path,
    timeout_seconds: u64,
) -> Result<(Value, Vec<String>, i64, String), ApiError> {
    let mut cmd = Command::new(claw_bin);
    cmd.current_dir(work_dir)
        .arg("mcp")
        .arg("--output-format")
        .arg("json")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = timeout(Duration::from_secs(timeout_seconds), cmd.output())
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                format!("claw mcp probe timeout: {timeout_seconds}s"),
            )
        })?
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("spawn claw mcp failed: {e}"),
            )
        })?;
    let raw = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).to_string()
    } else {
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    let parsed = serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| json!({"raw": raw}));
    let loaded_names = parsed
        .get("servers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("name")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let configured_servers = parsed
        .get("configured_servers")
        .and_then(Value::as_i64)
        .unwrap_or(loaded_names.len() as i64);
    let status = parsed
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(if output.status.success() {
            "ok"
        } else {
            "error"
        })
        .to_string();
    Ok((parsed, loaded_names, configured_servers, status))
}

async fn get_ds_lock(state: &AppState, ds_id: i64) -> Arc<Mutex<()>> {
    let mut locks = state.ds_locks.lock().await;
    locks
        .entry(ds_id)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

async fn validate_ds_exists(ds_id: i64, path: &Path) -> Result<(), ApiError> {
    if fs::metadata(path).await.is_err() {
        warn!("datasource registry not found: {}", path.display());
        return Ok(());
    }
    let text = fs::read_to_string(path).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read datasource registry failed: {e}"),
        )
    })?;
    let parsed = serde_yaml::from_str::<Value>(&text).unwrap_or_else(|_| json!({}));
    if let Some(ds) = parsed
        .get("datasources")
        .and_then(Value::as_object)
        .and_then(|m| m.get(&ds_id.to_string()))
    {
        if ds.is_object() {
            return Ok(());
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as i64
}

fn current_utc_date() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let days_since_epoch = (now.as_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days_since_epoch);
    format!("{year:04}-{month:02}-{day:02}")
}

// Computes civil (Gregorian) year/month/day from days since the Unix epoch
// (1970-01-01) using Howard Hinnant's `civil_from_days` algorithm.
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = (z - era * 146_097) as u64; // [0, 146_096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = y + i64::from(m <= 2);
    (y as i32, m as u32, d as u32)
}

fn load_mcp_servers_from_claw_config() -> HashMap<String, Value> {
    let Ok(path) = std::env::var("CLAW_CONFIG_FILE") else {
        return HashMap::new();
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return HashMap::new(),
    };
    let parsed = match serde_json::from_str::<Value>(&raw) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };
    let mut out = HashMap::new();
    let Some(mcp) = parsed.get("mcpServers").and_then(Value::as_object) else {
        return out;
    };
    for (name, cfg) in mcp {
        out.insert(name.clone(), cfg.clone());
    }
    out
}

fn parse_allowed_tools(raw: Option<String>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for token in raw.split(',') {
        let name = normalize_allowed_tool_name(token);
        if name.is_empty() {
            continue;
        }
        if !values.contains(&name) {
            values.push(name);
        }
    }
    values
}

fn normalize_allowed_tool_name(raw: &str) -> String {
    let name = raw.trim();
    match name {
        "read" | "ReadFile" | "ead_file" => "read_file".to_string(),
        "glob" | "GlobSearch" | "glob_searchr" => "glob_search".to_string(),
        "grep" | "GrepSearch" => "grep_search".to_string(),
        "MCPTool" => "MCP".to_string(),
        "ListMcpResourcesToolMCP" => "ListMcpResources".to_string(),
        other => other.to_string(),
    }
}

pub(crate) fn resolve_effective_allowed_tools(
    global_allowed_tools: &[String],
    requested_allowed_tools: Option<&[String]>,
) -> Result<Vec<String>, ApiError> {
    let Some(requested) = requested_allowed_tools else {
        return Ok(global_allowed_tools.to_vec());
    };

    let mut normalized = Vec::new();
    for raw in requested {
        let name = normalize_allowed_tool_name(raw);
        if name.is_empty() {
            continue;
        }
        if !normalized.contains(&name) {
            normalized.push(name);
        }
    }
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    if global_allowed_tools.is_empty() {
        return Ok(normalized);
    }

    for requested in &normalized {
        let allowed = if requested.ends_with('*') {
            global_allowed_tools.contains(requested)
        } else {
            is_tool_allowed(requested, global_allowed_tools)
        };
        if !allowed {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("requested tool pattern is not allowed by gateway policy: {requested}"),
            ));
        }
    }
    Ok(normalized)
}

fn is_tool_allowed(tool_name: &str, allowed_tools: &[String]) -> bool {
    if allowed_tools.is_empty() {
        return true;
    }
    for pattern in allowed_tools {
        if pattern == tool_name {
            return true;
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            if tool_name.starts_with(prefix) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_skill_name_accepts_expected_charset() {
        assert!(validate_skill_name("abc").is_ok());
        assert!(validate_skill_name("a-b_c.d").is_ok());
        assert!(validate_skill_name("Skill_01").is_ok());
    }

    #[test]
    fn validate_skill_name_rejects_empty_or_unsafe_names() {
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("   ").is_err());
        assert!(validate_skill_name("../escape").is_err());
        assert!(validate_skill_name("bad/name").is_err());
        assert!(validate_skill_name("中文").is_err());
    }

    #[test]
    fn ds_work_dir_and_claude_paths_match_contract() {
        let root = Path::new("/tmp/gateway-work");
        let work_dir = ds_work_dir(root, 27);
        assert_eq!(work_dir, PathBuf::from("/tmp/gateway-work/ds_27"));
        let (home_claude, root_claude) = project_claude_paths(&work_dir);
        assert_eq!(
            home_claude,
            PathBuf::from("/tmp/gateway-work/ds_27/home/CLAUDE.md")
        );
        assert_eq!(
            root_claude,
            PathBuf::from("/tmp/gateway-work/ds_27/CLAUDE.md")
        );
    }

    #[test]
    fn projects_git_effective_clone_url_inserts_github_pat() {
        let u = projects_git_effective_clone_url(
            "https://github.com/passionke/claw-code-projects.git",
            Some("ghp_secret"),
        );
        assert_eq!(
            u,
            "https://x-access-token:ghp_secret@github.com/passionke/claw-code-projects.git"
        );
    }

    #[test]
    fn projects_git_effective_clone_url_skips_injection_when_userinfo_present() {
        let u = projects_git_effective_clone_url(
            "https://user:pass@github.com/passionke/claw-code-projects.git",
            Some("ghp_secret"),
        );
        assert_eq!(
            u,
            "https://user:pass@github.com/passionke/claw-code-projects.git"
        );
    }

    #[test]
    fn projects_git_effective_clone_url_ssh_ignores_token() {
        let u = projects_git_effective_clone_url(
            "git@github.com:passionke/claw-code-projects.git",
            Some("ghp_secret"),
        );
        assert_eq!(u, "git@github.com:passionke/claw-code-projects.git");
    }

    #[test]
    fn projects_git_message_suggests_push_retry_detects_common_git_errors() {
        assert!(projects_git_message_suggests_push_retry(
            "error: failed to push some refs ... ! [rejected] ... (non-fast-forward)"
        ));
        assert!(projects_git_message_suggests_push_retry(
            "Updates were rejected because the remote contains work that you do not have locally."
        ));
        assert!(!projects_git_message_suggests_push_retry(
            "fatal: could not read Username"
        ));
    }

    #[test]
    fn parse_projects_git_author_splits_name_email() {
        let (n, e) = parse_projects_git_author("kejiqing <kejiqing@local>");
        assert_eq!(n, "kejiqing");
        assert_eq!(e, "kejiqing@local");
    }
}
