//! AppState and gateway config. Author: kejiqing
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{
    claw_tap_cluster_state, gateway_llm_config_sync, pool, session_db, session_ovs_api,
    session_terminal_api,
};
use gateway_solve_turn::ReportPolishDeepseek;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::task::AbortHandle;

type SessionSolveLocks = Arc<Mutex<HashMap<(i64, String), Arc<Mutex<()>>>>>;
type DockerSlots = Arc<Mutex<HashMap<String, (Arc<dyn pool::PoolOps + Send + Sync>, usize)>>>;

#[derive(Clone)]
pub(crate) struct HttpRequestId(pub String);

#[derive(Clone)]
pub(crate) struct RunSolveContext {
    pub(crate) request_id: String,
    pub(crate) task_id: Option<String>,
    /// Per-solve turn id (`T_<32 hex>`); persisted in `gateway_turns`.
    pub(crate) turn_id: String,
    /// When true, do not read/write the gateway session `SQLite` (e.g. internal biz report solve).
    pub(crate) skip_session_db: bool,
    /// Who enqueued this turn (`gateway-admin`, external app, …). Author: kejiqing
    pub(crate) client_origin: Option<String>,
}

#[allow(clippy::struct_field_names)]
pub(crate) struct PreparedGatewaySession {
    pub(crate) session_home: PathBuf,
    pub(crate) session_home_rel: String,
    pub(crate) session_fs_label: String,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) tasks: Arc<Mutex<HashMap<String, TaskInner>>>,
    pub(crate) injected_mcp: Arc<Mutex<HashMap<i64, HashMap<String, Value>>>>,
    pub(crate) proj_locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
    /// Serialize solve per `(proj_id, session_id)` for transcript + workspace safety.
    pub(crate) session_solve_locks: SessionSolveLocks,
    pub(crate) session_db: Arc<session_db::GatewaySessionDb>,
    pub(crate) cfg: Arc<GatewayConfig>,
    /// Active async task id → pool + slot for cancel (FC solve leases).
    pub(crate) docker_slots: DockerSlots,
    pub(crate) pool_clients: pool::PoolClients,
    /// Worker stdout `report.delta` ingest + live SSE for running turns. Author: kejiqing
    pub(crate) live_report_hub: Arc<pool::LiveReportHub>,
    /// Serialize git and working-tree reads/writes on the shared `.claw-code-projects` clone. kejiqing
    pub(crate) projects_git_mirror_lock: Arc<Mutex<()>>,
    /// Active LLM model/api key from DB (refreshed on apply + poll). Author: kejiqing
    pub(crate) llm_runtime: gateway_llm_config_sync::LlmRuntimeHandle,
    /// clawTap cluster consistency (strict only; mismatch blocks solve). Author: kejiqing
    pub(crate) claw_tap_cluster: claw_tap_cluster_state::ClawTapClusterHandle,
    /// Active interactive sessions for OVS `agent/ws` (internal ttyd bridge). Author: kejiqing
    pub(crate) terminal_registry: session_terminal_api::TerminalSessionRegistry,
    /// NAS layout + file writes via e2b claw-nas-api singleton (required in e2b mode).
    pub(crate) nas_api: Arc<pool::E2bNasApiSingleton>,
    /// This process's gateway ingress identity (multi-gateway same clusterId). Author: kejiqing
    pub(crate) gateway_identity: Arc<crate::gateway_endpoint::GatewayEndpointIdentity>,
}

impl AppState {
    #[must_use]
    pub(crate) fn terminal_api_ctx(&self) -> session_terminal_api::TerminalApiContext {
        session_terminal_api::terminal_api_context(
            self.cfg.work_root.clone(),
            self.cfg.pool_rpc_host_work_root.clone(),
            self.pool_clients.clone(),
            self.session_db.clone(),
            self.terminal_registry.clone(),
            container_runtime_bin(),
            self.claw_tap_cluster.clone(),
            self.llm_runtime.clone(),
        )
    }

    #[must_use]
    pub(crate) fn ovs_api_ctx(&self) -> session_ovs_api::OvsApiContext {
        session_ovs_api::ovs_api_context(self.cfg.work_root.clone())
    }
}

#[must_use]
pub(crate) fn container_runtime_bin() -> String {
    std::env::var("CLAW_CONTAINER_RUNTIME")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("auto"))
        .unwrap_or_else(|| "podman".to_string())
}

#[derive(Clone)]
pub(crate) struct GatewayConfig {
    pub(crate) claw_bin: String,
    pub(crate) work_root: PathBuf,
    /// Host `CLAW_WORK_ROOT` when gateway container paths differ from NAS bind source.
    pub(crate) pool_rpc_host_work_root: Option<PathBuf>,
    /// Same-machine pool id (`CLAW_POOL_ID` / hostname); written on turn enqueue for live SSE JOIN.
    pub(crate) co_located_pool_id: Option<String>,
    pub(crate) ds_registry_path: PathBuf,
    pub(crate) default_timeout_seconds: u64,
    pub(crate) default_max_iterations: usize,
    pub(crate) default_http_mcp_name: Option<String>,
    pub(crate) default_http_mcp_url: Option<String>,
    pub(crate) default_http_mcp_transport: String,
    /// Remote URL for `claw-code-projects` mirror (SSH or HTTPS; no embedded token).
    pub(crate) projects_git_url: String,
    pub(crate) projects_git_branch: String,
    /// Passed to `git commit --author`.
    pub(crate) projects_git_author: String,
    /// When set with an `https://` or credential-less `http://` `projects_git_url`, used for clone/pull/push (injected as `x-access-token` user; GitHub-compatible; GitLab may need userinfo URL).
    pub(crate) projects_git_token: Option<String>,
    /// When set, periodically `git pull` the mirror and refresh each `ds_*/home` when that ds lock is idle (multi-node). kejiqing
    pub(crate) projects_git_proj_home_poll_interval_secs: Option<u64>,
    /// Poll DB active LLM → upstream JSON file + in-memory runtime (0 = disabled). Author: kejiqing
    pub(crate) gateway_llm_config_poll_interval_secs: Option<u64>,
    /// When set (`REPORT_LLM_PROVIDER=deepseek` + `DEEPSEEK_API_KEY`), `/v1/biz_advice_report` polish calls `DeepSeek` official API. kejiqing
    pub(crate) report_polish_deepseek: Option<ReportPolishDeepseek>,
    /// `CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL=1`: legacy BOSS report — `hasReport` only when `succeeded`; report SSE = LLM polish (no pool live proxy). Author: kejiqing
    pub(crate) live_biz_report_spill_enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct SolveRequest {
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    pub(crate) proj_id: i64,
    #[serde(rename = "userPrompt")]
    pub(crate) user_prompt: String,
    /// When set, continue an existing gateway session for this `dsId` (must exist in session DB).
    #[serde(default, rename = "sessionId")]
    pub(crate) session_id: Option<String>,
    pub(crate) model: Option<String>,
    #[serde(rename = "timeoutSeconds")]
    pub(crate) timeout_seconds: Option<u64>,
    #[serde(rename = "extraSession")]
    #[schema(value_type = Object)]
    pub(crate) extra_session: Option<Value>,
    #[serde(rename = "allowedTools")]
    pub(crate) allowed_tools: Option<Vec<String>>,
    /// Session-relative attachments (uploaded via `/v1/sessions/{id}/files`). Author: kejiqing
    #[serde(default, rename = "attachments")]
    #[schema(value_type = Option<Vec<Object>>)]
    pub(crate) attachments: Option<Vec<gateway_solve_turn::SolveAttachment>>,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
pub(crate) struct SolveResponse {
    #[serde(rename = "sessionId")]
    pub(crate) session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    pub(crate) request_id: String,
    /// Relative to `CLAW_WORK_ROOT` (matches DB `gateway_sessions.session_home`). kejiqing
    #[serde(rename = "sessionHomeRel")]
    pub(crate) session_home_rel: String,
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    pub(crate) proj_id: i64,
    #[serde(rename = "workDir")]
    pub(crate) work_dir: String,
    #[serde(rename = "durationMs")]
    pub(crate) duration_ms: i64,
    #[serde(rename = "clawExitCode")]
    pub(crate) claw_exit_code: i32,
    #[serde(rename = "outputText")]
    pub(crate) output_text: String,
    #[serde(rename = "outputJson")]
    #[schema(value_type = Object)]
    pub(crate) output_json: Option<Value>,
    #[serde(rename = "turnId")]
    pub(crate) turn_id: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct SolveAsyncResponse {
    #[serde(rename = "taskId")]
    pub(crate) task_id: String,
    #[serde(rename = "sessionId")]
    pub(crate) session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    pub(crate) request_id: String,
    #[serde(rename = "turnId")]
    pub(crate) turn_id: String,
    pub(crate) status: String,
    #[serde(rename = "pollUrl")]
    pub(crate) poll_url: String,
    #[serde(rename = "poolId", skip_serializing_if = "Option::is_none")]
    pub(crate) pool_id: Option<String>,
    #[serde(rename = "workerName", skip_serializing_if = "Option::is_none")]
    pub(crate) worker_name: Option<String>,
    /// Requested ds isolation (`project_config.worker_profile_json.mode`). Author: kejiqing
    #[serde(rename = "workerProfile", skip_serializing_if = "Option::is_none")]
    pub(crate) worker_profile: Option<String>,
    /// Actual `podman exec --user` on the pool (`claw`, etc.). Author: kejiqing
    #[serde(rename = "workerExecUser", skip_serializing_if = "Option::is_none")]
    pub(crate) worker_exec_user: Option<String>,
    #[serde(rename = "gatewayId", skip_serializing_if = "Option::is_none")]
    pub(crate) gateway_id: Option<String>,
    #[serde(rename = "gatewayBase", skip_serializing_if = "Option::is_none")]
    pub(crate) gateway_base: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct StartRequest {
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    pub(crate) proj_id: i64,
    /// When set, continue an existing gateway session for this `dsId` (must exist in session DB).
    #[serde(default, rename = "sessionId")]
    pub(crate) session_id: Option<String>,
    #[serde(default, rename = "extraSession")]
    #[schema(value_type = Object)]
    pub(crate) extra_session: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct SolveStartResponse {
    #[serde(rename = "sessionId")]
    pub(crate) session_id: String,
    #[serde(rename = "requestId")]
    pub(crate) request_id: String,
}
pub(crate) struct TaskInner {
    pub(crate) record: TaskRecord,
    /// Present while `queued` / `running`; cleared when the worker finishes or after cancel.
    pub(crate) cancel: Option<AbortHandle>,
    pub(crate) proj_id: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
pub(crate) struct TaskRecord {
    #[serde(rename = "taskId")]
    pub(crate) task_id: String,
    #[serde(rename = "sessionId")]
    pub(crate) session_id: String,
    // Backward-compat field; keep in sync with sessionId.
    #[serde(rename = "requestId")]
    pub(crate) request_id: String,
    #[serde(rename = "projId", alias = "proj_id", alias = "dsId", alias = "ds_id")]
    pub(crate) proj_id: i64,
    pub(crate) status: String,
    #[serde(rename = "createdAtMs")]
    pub(crate) created_at_ms: i64,
    #[serde(rename = "startedAtMs")]
    pub(crate) started_at_ms: Option<i64>,
    #[serde(rename = "finishedAtMs")]
    pub(crate) finished_at_ms: Option<i64>,
    #[serde(rename = "currentTaskDesc", skip_serializing_if = "Option::is_none")]
    pub(crate) current_task_desc: Option<String>,
    #[serde(
        rename = "progressUpdatedAtMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub(crate) progress_updated_at_ms: Option<i64>,
    pub(crate) result: Option<SolveResponse>,
    #[schema(value_type = Object)]
    pub(crate) error: Option<Value>,
    #[serde(rename = "turnId")]
    pub(crate) turn_id: String,
    #[serde(
        rename = "progressHistory",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    #[schema(value_type = Vec<Object>)]
    pub(crate) progress_history: Vec<gateway_solve_turn::ProgressEvent>,
    /// `true` after first `report.delta` is observed (or terminal persisted report). Author: kejiqing
    #[serde(rename = "hasReport")]
    pub(crate) has_report: bool,
    /// First report material time (ms): stdout hub first delta, else `startedAtMs` / `finishedAtMs`.
    #[serde(rename = "reportTime", skip_serializing_if = "Option::is_none")]
    pub(crate) report_time_ms: Option<i64>,
    #[serde(rename = "planTitle", skip_serializing_if = "Option::is_none")]
    pub(crate) plan_title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schema(value_type = Vec<Object>)]
    pub(crate) todos: Vec<gateway_solve_turn::TaskProgressTodo>,
    #[serde(rename = "poolId", skip_serializing_if = "Option::is_none")]
    pub(crate) pool_id: Option<String>,
    #[serde(rename = "workerName", skip_serializing_if = "Option::is_none")]
    pub(crate) worker_name: Option<String>,
    #[serde(rename = "workerProfile", skip_serializing_if = "Option::is_none")]
    pub(crate) worker_profile: Option<String>,
    #[serde(rename = "workerExecUser", skip_serializing_if = "Option::is_none")]
    pub(crate) worker_exec_user: Option<String>,
    #[serde(rename = "gatewayId", skip_serializing_if = "Option::is_none")]
    pub(crate) gateway_id: Option<String>,
    #[serde(rename = "gatewayBase", skip_serializing_if = "Option::is_none")]
    pub(crate) gateway_base: Option<String>,
}
