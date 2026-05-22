//! Axum gateway: single-binary integration surface (keeps clippy noise localized).
#![recursion_limit = "256"]
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
#![allow(clippy::similar_names)]

mod project_config_draft;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Extension, Path as AxumPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{AppendHeaders, Html, IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use biz_advice_report::{
    biz_report_sse_event_stream, build_biz_advice_polish_prompt,
    load_boss_report_writer_instructions, report_body_from_solve_output, sanitize_biz_report_parts,
    sanitize_external_report_text, sanitize_report_payload, BizAdviceReportPayload,
    BizReportStreamMsg, ReportExportSanitizer,
};
use biz_advice_report_live::{
    should_use_live_pg_report, spawn_live_report_sse_worker, LiveReportContext,
};
use gateway_solve_turn::read_progress_events;
use gateway_solve_turn::{
    read_task_progress, reset_task_progress, run_gateway_biz_polish_llm,
    run_gateway_biz_polish_llm_async, truncate_progress_history, ReportPolishDeepseek,
    BOSS_REPORT_SKILL_DS_ID,
};
use http_gateway_rs::{
    gateway_global_settings, project_config_apply, project_config_version, project_entity_revision,
    project_git_sync, project_tools, session_db, session_merge, turn_id,
};
use project_git_sync::{git_sync_list_summary, git_sync_to_json, parse_git_sync_json, GitPushOutcome};
use runtime::load_system_prompt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use session_execution::{
    discover_trace_paths, join_session_home, read_trace_tail, trace_tail_suggests_tool_call,
    SessionExecutionResponse, SessionExecutionTask,
};
use task_status::{
    count_gateway_tasks, ensure_report_progress_in_allowed_tools, resolve_current_task_desc,
    TaskStatusRow,
};
use tokio::fs;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio::task::AbortHandle;
use tokio::time::{interval, timeout, MissedTickBehavior};
use tower_http::trace::TraceLayer;
use tracing::field::Empty;
use tracing::{info, warn};
use uuid::Uuid;

mod biz_advice_report;
mod biz_advice_report_live;
mod gateway_logging;
mod pool;
mod session_execution;
mod solve_pool;
mod task_status;
mod turn_live;
mod live_report_ports;

#[cfg(test)]
mod live_report_mocks;

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
    /// Per-solve turn id (`T_<32 hex>`); persisted in `gateway_turns`.
    turn_id: String,
    /// When true, do not read/write the gateway session `SQLite` (e.g. internal biz report solve).
    skip_session_db: bool,
}

/// Session workspace paths after sync registry prepare (before docker solve). kejiqing
#[allow(clippy::struct_field_names)]
struct PreparedGatewaySession {
    session_home: PathBuf,
    session_home_rel: String,
    session_fs_label: String,
}

#[derive(Clone)]
pub(crate) struct AppState {
    tasks: Arc<Mutex<HashMap<String, TaskInner>>>,
    injected_mcp: Arc<Mutex<HashMap<i64, HashMap<String, Value>>>>,
    ds_locks: Arc<Mutex<HashMap<i64, Arc<Mutex<()>>>>>,
    /// Serialize solve per `(ds_id, session_id)` for transcript + workspace safety.
    session_solve_locks: Arc<Mutex<HashMap<(i64, String), Arc<Mutex<()>>>>>,
    session_db: Arc<session_db::GatewaySessionDb>,
    cfg: Arc<GatewayConfig>,
    /// When using `docker_pool` / `podman_pool`, active async task id → pool + slot for cancel.
    docker_slots: Arc<Mutex<HashMap<String, (Arc<dyn pool::PoolOps + Send + Sync>, usize)>>>,
    docker_pool: Arc<dyn pool::PoolOps + Send + Sync>,
    /// Serialize git and working-tree reads/writes on the shared `.claw-code-projects` clone. kejiqing
    projects_git_mirror_lock: Arc<Mutex<()>>,
    live_ingest_closed: turn_live::LiveIngestRegistry,
    live_notify_hub: Arc<turn_live::LiveNotifyHub>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SolveIsolation {
    DockerPool,
    PodmanPool,
}

impl SolveIsolation {
    fn from_env() -> Self {
        let raw = std::env::var("CLAW_SOLVE_ISOLATION")
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_default();
        match raw.as_str() {
            "" | "podman_pool" => Self::PodmanPool,
            "docker_pool" => Self::DockerPool,
            "inprocess" => {
                eprintln!(
                    "http-gateway-rs: CLAW_SOLVE_ISOLATION=inprocess is removed; use podman_pool or docker_pool."
                );
                std::process::exit(1);
            }
            other => {
                eprintln!(
                    "http-gateway-rs: invalid CLAW_SOLVE_ISOLATION={other:?}; expected podman_pool or docker_pool."
                );
                std::process::exit(1);
            }
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::DockerPool => "docker_pool",
            Self::PodmanPool => "podman_pool",
        }
    }
}

#[derive(Clone)]
pub(crate) struct GatewayConfig {
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
    /// `CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL=1`: solve 写 spill、`hasReport` 提前、报告 SSE tail；默认关 → 仅 LLM 润色。
    live_biz_report_spill_enabled: bool,
    default_http_mcp_name: Option<String>,
    default_http_mcp_url: Option<String>,
    default_http_mcp_transport: String,
    config_mcp_servers: HashMap<String, Value>,
    /// Remote URL for `claw-code-projects` mirror (SSH or HTTPS; no embedded token).
    projects_git_url: String,
    projects_git_branch: String,
    /// Passed to `git commit --author`.
    projects_git_author: String,
    /// When set with an `https://` or credential-less `http://` `projects_git_url`, used for clone/pull/push (injected as `x-access-token` user; GitHub-compatible; GitLab may need userinfo URL).
    projects_git_token: Option<String>,
    /// When set, periodically `git pull` the mirror and refresh each `ds_*/home` when that ds lock is idle (multi-node). kejiqing
    projects_git_ds_home_poll_interval_secs: Option<u64>,
    /// When set (`REPORT_LLM_PROVIDER=deepseek` + `DEEPSEEK_API_KEY`), `/v1/biz_advice_report` polish calls `DeepSeek` official API. kejiqing
    report_polish_deepseek: Option<ReportPolishDeepseek>,
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
    /// Per-request override for spill file (`CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL` is the gateway default when omitted).
    #[serde(rename = "assistantStreamSpill", default)]
    assistant_stream_spill: Option<bool>,
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
    #[serde(rename = "turnId")]
    turn_id: String,
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
    #[serde(rename = "turnId")]
    turn_id: String,
    status: String,
    #[serde(rename = "pollUrl")]
    poll_url: String,
}

/// Session bootstrap (`POST /v1/start`): sync `SQLite` + workspace only (no solve). kejiqing
#[derive(Debug, Serialize, Deserialize)]
struct StartRequest {
    #[serde(rename = "dsId")]
    ds_id: i64,
    /// When set, continue an existing gateway session for this `dsId` (must exist in session DB).
    #[serde(default, rename = "sessionId")]
    session_id: Option<String>,
    #[serde(default, rename = "extraSession")]
    extra_session: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SolveStartResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "requestId")]
    request_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InitRequest {
    #[serde(rename = "dsId")]
    ds_id: i64,
}

/// `POST /v1/projects` — create `ds_<id>` workspace (+ optional projects-git push). Author: kejiqing
#[derive(Debug, Deserialize)]
struct CreateProjectRequest {
    #[serde(rename = "dsId")]
    ds_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DeleteProjectQuery {
    #[serde(default = "default_true")]
    purge_sessions: bool,
}

fn default_true() -> bool {
    true
}

/// `GET /v1/projects` — list `project_config` from PostgreSQL + disk overlay. Author: kejiqing
#[derive(Debug, Serialize)]
struct ProjectListEntry {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "contentRev")]
    content_rev: String,
    #[serde(rename = "draftOpen")]
    draft_open: bool,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: i64,
    #[serde(rename = "skillsCountDb")]
    skills_count_db: i64,
    #[serde(rename = "claudeInDb")]
    claude_in_db: bool,
    #[serde(rename = "rulesCountDb")]
    rules_count_db: i64,
    #[serde(rename = "mcpServersCountDb")]
    mcp_servers_count_db: i64,
    #[serde(rename = "workDirPresent")]
    work_dir_present: bool,
    #[serde(rename = "environmentPrepared")]
    environment_prepared: bool,
    #[serde(rename = "claudeOnDisk")]
    claude_on_disk: bool,
    #[serde(rename = "skillsCountDisk")]
    skills_count_disk: u64,
    #[serde(rename = "appliedRev")]
    applied_rev: Option<String>,
    #[serde(rename = "dbSyncedToDisk")]
    db_synced_to_disk: bool,
    /// Per-project one-way git (no PAT in list). Author: kejiqing
    #[serde(rename = "gitSync")]
    git_sync: Value,
}

#[derive(Debug, Serialize)]
struct ProjectListResponse {
    projects: Vec<ProjectListEntry>,
    #[serde(rename = "listedAtMs")]
    listed_at_ms: i64,
}

#[derive(Debug, Serialize)]
struct DeleteProjectResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    deleted: bool,
    #[serde(rename = "purgeSessions")]
    purge_sessions: bool,
    #[serde(rename = "sessionsRemoved")]
    sessions_removed: u64,
    #[serde(rename = "projectConfigRemoved")]
    project_config_removed: bool,
    #[serde(rename = "gitSync", skip_serializing_if = "Option::is_none")]
    git_sync: Option<GitSyncResponse>,
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

/// Body for `PUT /v1/project/config/{ds_id}` — writes the open draft only. Author: kejiqing
#[derive(Debug, Deserialize)]
struct UpsertProjectConfigRequest {
    #[serde(rename = "contentRev", default)]
    content_rev: String,
    #[serde(rename = "rulesJson", default)]
    rules_json: Value,
    #[serde(rename = "mcpServersJson", default)]
    mcp_servers_json: Value,
    #[serde(rename = "skillsSourcesJson", default)]
    skills_sources_json: Value,
    #[serde(rename = "skillsJson", default)]
    skills_json: Value,
    #[serde(rename = "allowedToolsJson", default)]
    allowed_tools_json: Value,
    #[serde(rename = "claudeMd")]
    claude_md: Option<String>,
    /// Omit on PUT to keep existing `git_sync_json`. Author: kejiqing
    #[serde(rename = "gitSyncJson", default)]
    git_sync_json: Option<Value>,
}

/// Body for `POST /v1/project/config/{ds_id}/versions/commit` — save draft as immutable formal revision (does not change effective). Author: kejiqing
#[derive(Debug, Deserialize)]
struct CommitProjectConfigDraftRequest {
    /// Optional label; version id is auto-generated (`YYYYMMDDHHmmss` local). Author: kejiqing
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProjectToolsCatalogResponse {
    tools: Vec<project_tools::ToolCatalogEntry>,
}

#[derive(Debug, Serialize)]
struct ProjectConfigResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "contentRev")]
    content_rev: String,
    #[serde(rename = "stableContentRev", skip_serializing_if = "Option::is_none")]
    stable_content_rev: Option<String>,
    #[serde(rename = "draftOpen")]
    draft_open: bool,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: i64,
    #[serde(rename = "rulesJson")]
    rules_json: Value,
    #[serde(rename = "mcpServersJson")]
    mcp_servers_json: Value,
    #[serde(rename = "skillsSourcesJson")]
    skills_sources_json: Value,
    #[serde(rename = "skillsJson")]
    skills_json: Value,
    #[serde(rename = "allowedToolsJson")]
    allowed_tools_json: Value,
    #[serde(rename = "claudeMd")]
    claude_md: Option<String>,
    #[serde(rename = "gitSyncJson")]
    git_sync_json: Value,
}

#[derive(Debug, Serialize)]
struct ProjectConfigVersionsResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    /// Effective formal revision id (one of non-draft rows in `versions`).
    #[serde(rename = "activeContentRev")]
    active_content_rev: String,
    #[serde(rename = "appliedContentRev", skip_serializing_if = "Option::is_none")]
    applied_content_rev: Option<String>,
    #[serde(rename = "draftOpen")]
    draft_open: bool,
    /// Formal revisions plus optional single `__draft__` row when `draftOpen`.
    versions: Vec<ProjectConfigVersionEntry>,
}

#[derive(Debug, Serialize)]
struct ProjectConfigVersionEntry {
    #[serde(rename = "contentRev")]
    content_rev: String,
    #[serde(rename = "createdAtMs")]
    created_at_ms: i64,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "note", skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(rename = "isActive")]
    is_active: bool,
    #[serde(rename = "claudeInDb")]
    claude_in_db: bool,
    #[serde(rename = "skillsCountDb")]
    skills_count_db: i64,
    #[serde(rename = "rulesCountDb")]
    rules_count_db: i64,
    #[serde(rename = "mcpServersCountDb")]
    mcp_servers_count_db: i64,
}

#[derive(Debug, Deserialize)]
struct CompareProjectConfigQuery {
    from: String,
    to: String,
}

#[derive(Debug, Serialize)]
struct ActivateProjectConfigVersionResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "activeContentRev")]
    active_content_rev: String,
    activated: bool,
    #[serde(rename = "materialized")]
    materialized: bool,
}

#[derive(Debug, Serialize)]
struct ProjectGitPushResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    outcome: GitPushOutcome,
    #[serde(rename = "gitSyncJson")]
    git_sync_json: Value,
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
}

#[derive(Debug, Serialize)]
struct EffectivePromptResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "workDir")]
    work_dir: String,
    sections: Vec<String>,
    message: String,
    /// `user` = project `claudeMd` override only; `system` = DB scaffold + project context.
    #[serde(rename = "promptSource")]
    prompt_source: String,
}

/// Per-datasource skill files under `<work_root>/ds_<id>/home/skills/<name>/SKILL.md` (same tree as `POST /v1/project/skills`). kejiqing
#[derive(Debug, Serialize)]
struct DsSkillEntry {
    skill_name: String,
    skill_content: String,
}

#[derive(Debug, Serialize)]
struct DsSkillsListResponse {
    ds_id: i64,
    skills: Vec<DsSkillEntry>,
}

#[derive(Debug, Serialize)]
struct DsSkillGetResponse {
    ds_id: i64,
    skill_name: String,
    skill_content: String,
}

/// In-memory task row plus a handle to abort the async worker (not serialized). kejiqing
struct TaskInner {
    record: TaskRecord,
    /// Present while `queued` / `running`; cleared when the worker finishes or after cancel.
    cancel: Option<AbortHandle>,
    ds_id: i64,
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
    #[serde(rename = "dsId")]
    ds_id: i64,
    status: String,
    #[serde(rename = "createdAtMs")]
    created_at_ms: i64,
    #[serde(rename = "startedAtMs")]
    started_at_ms: Option<i64>,
    #[serde(rename = "finishedAtMs")]
    finished_at_ms: Option<i64>,
    #[serde(rename = "currentTaskDesc", skip_serializing_if = "Option::is_none")]
    current_task_desc: Option<String>,
    #[serde(
        rename = "progressUpdatedAtMs",
        skip_serializing_if = "Option::is_none"
    )]
    progress_updated_at_ms: Option<i64>,
    result: Option<SolveResponse>,
    error: Option<Value>,
    #[serde(rename = "turnId")]
    turn_id: String,
    #[serde(
        rename = "progressHistory",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    progress_history: Vec<gateway_solve_turn::ProgressEvent>,
    /// `true` when succeeded, or while running once spill/result contains `__CLAW_REPORT_START__`.
    #[serde(rename = "hasReport")]
    has_report: bool,
}

#[derive(Debug, Deserialize)]
struct AgentFeedbackPostRequest {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "turnId")]
    turn_id: String,
    feedback: String,
}

#[derive(Debug, Deserialize)]
struct AgentFeedbackGetQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "dsId")]
    ds_id: Option<i64>,
    #[serde(rename = "ds_id")]
    ds_id_alt: Option<i64>,
}

impl AgentFeedbackGetQuery {
    fn resolved_ds_id(&self) -> Option<i64> {
        self.ds_id.or(self.ds_id_alt)
    }
}

#[derive(Debug, Serialize)]
struct AgentFeedbackPostResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "turnId")]
    turn_id: String,
    feedback: String,
    #[serde(rename = "updatedAtMs")]
    updated_at_ms: i64,
}

#[derive(Debug, Serialize)]
struct AgentFeedbackGetResponse {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "dsId")]
    ds_id: i64,
    items: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct ProbeQuery {
    #[serde(rename = "probe_timeout_seconds")]
    probe_timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct BizAdviceReportBakQuery {
    task_id: String,
    /// `true` 时返回 `text/event-stream`（`biz.report.start` / `delta` / `done`），走 LLM 润色。
    #[serde(default)]
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct BizAdviceReportQuery {
    #[serde(rename = "sessionId")]
    session_id: String,
    #[serde(rename = "turnId")]
    turn_id: String,
    #[serde(rename = "dsId")]
    ds_id: i64,
    /// `true`（默认）时 tail `.claw/assistant-stream-spill-{turnId}.txt` 并 SSE；结束后用 session jsonl 全量。
    #[serde(default = "default_biz_report_stream")]
    stream: bool,
}

fn default_biz_report_stream() -> bool {
    true
}

/// Dev-only: inject a succeeded task so `GET /v1/biz_advice_report` can run without `solve_async`.
/// Enable with `CLAW_GATEWAY_DEV_BIZ_REPORT_SEED=1`. Author: kejiqing
#[derive(Debug, Deserialize)]
struct DevBizReportSeedRequest {
    #[serde(rename = "taskId")]
    task_id: Option<String>,
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "outputText", default)]
    output_text: String,
    #[serde(rename = "outputJson")]
    output_json: Option<Value>,
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
pub(crate) struct ApiError {
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

async fn register_solve_turn(
    db: &session_db::GatewaySessionDb,
    turn_id: &str,
    session_id: &str,
    ds_id: i64,
    user_prompt: &str,
) -> Result<(), ApiError> {
    let prompt = user_prompt.trim();
    let user_prompt = (!prompt.is_empty()).then_some(prompt);
    db.insert_turn(turn_id, session_id, ds_id, "queued", now_ms(), user_prompt)
        .await
        .map_err(|e| session_db_err(&e))
}

async fn set_solve_turn_status(
    db: &session_db::GatewaySessionDb,
    turn_id: &str,
    status: &str,
    finished: bool,
) {
    let finished_at = finished.then_some(now_ms());
    if let Err(e) = db.update_turn_status(turn_id, status, finished_at).await {
        warn!(turn_id = %turn_id, error = %e, "update gateway_turns status failed");
    }
}

/// Persist terminal solve outcome on `gateway_turns` for restart / `GET /v1/tasks` handoff. Author: kejiqing
async fn finalize_solve_turn_success(
    db: Arc<session_db::GatewaySessionDb>,
    turn_id: &str,
    result: &SolveResponse,
) {
    let finished_at = Some(now_ms());
    let report =
        report_body_from_solve_output(&result.output_text, result.output_json.as_ref()).ok();
    if let Err(e) = db
        .finalize_turn_terminal(
            turn_id,
            "succeeded",
            finished_at,
            report.as_deref(),
            result.output_json.as_ref(),
            Some(result.claw_exit_code),
        )
        .await
    {
        warn!(
            turn_id = %turn_id,
            error = %e,
            "finalize gateway_turns succeeded snapshot failed"
        );
        return;
    }
    if report.as_ref().is_some_and(|t| !t.trim().is_empty()) {
        if let Err(e) = db.notify_turn_live_terminal(turn_id).await {
            warn!(
                turn_id = %turn_id,
                error = %e,
                "notify_turn_live_terminal failed"
            );
        }
        let db_del = db.clone();
        let tid = turn_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = db_del.delete_live_chunks(&tid).await {
                warn!(
                    turn_id = %tid,
                    error = %e,
                    "delete_live_chunks after succeeded failed"
                );
            }
        });
    }
}

async fn finalize_solve_turn_failed(
    db: &session_db::GatewaySessionDb,
    turn_id: &str,
    err: &ApiError,
) {
    let detail = json!({"status_code": err.status.as_u16(), "detail": err.message});
    if let Err(e) = db
        .finalize_turn_terminal(turn_id, "failed", Some(now_ms()), None, Some(&detail), None)
        .await
    {
        warn!(
            turn_id = %turn_id,
            error = %e,
            "finalize gateway_turns failed snapshot failed"
        );
    }
}

async fn finalize_solve_turn_cancelled(db: &session_db::GatewaySessionDb, turn_id: &str) {
    if let Err(e) = db
        .finalize_turn_terminal(turn_id, "cancelled", Some(now_ms()), None, None, None)
        .await
    {
        warn!(
            turn_id = %turn_id,
            error = %e,
            "finalize gateway_turns cancelled snapshot failed"
        );
    }
}

fn validate_feedback_value(feedback: &str) -> Result<(), ApiError> {
    if feedback == "good" || feedback == "bad" {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "feedback must be good or bad",
        ))
    }
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
fn pool_host_bind_root(work_root: &Path) -> PathBuf {
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
    let base = url.trim();
    let needs_creds = base.starts_with("https://") || base.starts_with("http://");
    if !needs_creds {
        return;
    }
    let rest = base
        .strip_prefix("https://")
        .or_else(|| base.strip_prefix("http://"))
        .unwrap_or("");
    let has_userinfo = rest.contains('@');
    let has_token = token.is_some_and(|t| !t.trim().is_empty());
    if !has_userinfo && !has_token {
        eprintln!(
            "http-gateway-rs: CLAW_PROJECTS_GIT_URL is HTTP(S) without embedded credentials (no userinfo before host) and CLAW_PROJECTS_GIT_TOKEN is unset or empty; set CLAW_PROJECTS_GIT_TOKEN or embed user:token@ in the URL."
        );
        std::process::exit(1);
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
    let pool_binding_root = pool_host_bind_root(&work_root);
    info!(
        target: "claw_gateway_orchestration",
        component = "startup",
        phase = "pool_host_paths",
        work_root = %work_root.display(),
        pool_host_bind_root = %pool_binding_root.display(),
        "container pool uses pool_host_bind_root on the runtime host for worker -v mounts"
    );
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

    let docker_pool: Arc<dyn pool::PoolOps + Send + Sync> = if let Some(ref tcp_addr) =
        pool_daemon_tcp
    {
        if pool_rpc_host_work_root.is_none() {
            warn!(
                target: "claw_gateway_orchestration",
                component = "startup",
                phase = "pool_rpc_missing_host_root",
                "CLAW_POOL_DAEMON_TCP is set but CLAW_POOL_RPC_HOST_WORK_ROOT is empty; acquire paths may not match the host daemon"
            );
        }
        let client = pool::PoolRpcClient::new_tcp(tcp_addr.clone());
        Arc::new(client)
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
        Arc::new(client)
    } else {
        let podman = matches!(solve_isolation, SolveIsolation::PodmanPool);
        let p =
            pool::DockerPoolManager::try_from_env(podman, &pool_binding_root).unwrap_or_else(|e| {
                let runtime = if podman { "Podman" } else { "Docker" };
                eprintln!("http-gateway-rs: invalid {runtime} pool configuration: {e}");
                std::process::exit(1);
            });
        pool::DockerPoolManager::schedule_warm(&p);
        Arc::new(pool::LocalPoolOps(p))
    };

    let projects_git_url = std::env::var("CLAW_PROJECTS_GIT_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_default();
    let projects_git_branch = std::env::var("CLAW_PROJECTS_GIT_BRANCH")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "main".to_string());
    let projects_git_author = std::env::var("CLAW_PROJECTS_GIT_AUTHOR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "claw-gateway <noreply@claw.local>".to_string());
    let projects_git_token = std::env::var("CLAW_PROJECTS_GIT_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if !projects_git_url.is_empty() {
        validate_projects_git_at_startup(&projects_git_url, projects_git_token.as_deref());
    }

    let report_polish_deepseek = {
        let raw = std::env::var("REPORT_LLM_PROVIDER")
            .ok()
            .map(|v| v.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        match raw.as_deref() {
            None | Some("") => None,
            Some("deepseek") => {
                let api_key = std::env::var("DEEPSEEK_API_KEY")
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|s| !s.is_empty());
                if let Some(api_key) = api_key {
                    let model = std::env::var("REPORT_DEEPSEEK_MODEL")
                        .ok()
                        .map(|v| v.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "deepseek-v4-pro".to_string());
                    info!(
                        target: "claw_gateway_orchestration",
                        component = "startup",
                        phase = "report_llm",
                        provider = "deepseek",
                        model = %model,
                        "biz_advice_report polish routes to DeepSeek official API (DEEPSEEK_BASE_URL or default)"
                    );
                    Some(ReportPolishDeepseek { api_key, model })
                } else {
                    warn!(
                        target: "claw_gateway_orchestration",
                        component = "startup",
                        phase = "report_llm",
                        "REPORT_LLM_PROVIDER=deepseek but DEEPSEEK_API_KEY is empty; using default report LLM routing"
                    );
                    None
                }
            }
            Some(other) => {
                warn!(
                    target: "claw_gateway_orchestration",
                    component = "startup",
                    phase = "report_llm",
                    provider = %other,
                    "unknown REPORT_LLM_PROVIDER; expected unset or deepseek; using default report LLM routing"
                );
                None
            }
        }
    };

    let cfg = GatewayConfig {
        solve_isolation,
        claw_bin: std::env::var("CLAW_BIN").unwrap_or_else(|_| "claw".to_string()),
        work_root,
        pool_rpc_host_work_root,
        pool_rpc_tcp: pool_rpc_tcp_cfg,
        pool_rpc_unix_socket: pool_rpc_unix_cfg,
        pool_rpc_remote: pool_daemon_tcp.is_some() || pool_daemon_socket.is_some(),
        ds_registry_path: std::env::var("CLAW_DS_REGISTRY").map_or_else(
            |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("datasources.example.yaml"),
            PathBuf::from,
        ),
        default_timeout_seconds: std::env::var("CLAW_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120),
        default_max_iterations: std::env::var("CLAW_MAX_ITERATIONS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(64),
        live_biz_report_spill_enabled: gateway_env_enabled("CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL"),
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
        projects_git_url,
        projects_git_branch,
        projects_git_author,
        projects_git_token,
        projects_git_ds_home_poll_interval_secs: std::env::var("CLAW_PROJECT_CONFIG_POLL_INTERVAL_SECS")
            .or_else(|_| std::env::var("CLAW_PROJECTS_GIT_DS_HOME_POLL_INTERVAL_SECS"))
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&s| s > 0),
        report_polish_deepseek,
    };
    let session_db = session_db::GatewaySessionDb::open()
        .await
        .unwrap_or_else(|e| {
            eprintln!(
                "http-gateway-rs: failed to connect gateway PostgreSQL (CLAW_GATEWAY_DATABASE_URL): {e}"
            );
            std::process::exit(1);
        });
    match session_db
        .reconcile_interrupted_turns_on_startup(now_ms())
        .await
    {
        Ok(n) if n > 0 => {
            info!(
                target: "claw_gateway_orchestration",
                component = "startup",
                phase = "session_db_reconcile",
                reconciled_turn_rows = n,
                "marked in-flight gateway_turns as failed after gateway restart"
            );
        }
        Ok(_) => {}
        Err(e) => warn!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "session_db_reconcile",
            error = %e,
            "reconcile_interrupted_turns_on_startup failed"
        ),
    }
    let session_db = Arc::new(session_db);
    info!(
        target: "claw_gateway_orchestration",
        component = "startup",
        phase = "session_db",
        gateway_database_url = %session_db.database_url_redacted(),
        "gateway session PostgreSQL ready (CLAW_GATEWAY_DATABASE_URL)"
    );
    let live_notify_hub = Arc::new(turn_live::LiveNotifyHub::new());
    if let Ok(db_url) = std::env::var("CLAW_GATEWAY_DATABASE_URL") {
        turn_live::LiveNotifyHub::spawn_listener(db_url, Arc::clone(&live_notify_hub));
    }
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
        live_ingest_closed: turn_live::LiveIngestRegistry::default(),
        live_notify_hub,
    };

    run_startup_project_config_apply(&state).await;

    if let Some(secs) = state.cfg.projects_git_ds_home_poll_interval_secs {
        let poller_state = state.clone();
        tokio::spawn(async move { project_config_poll_loop(poller_state, secs).await });
        info!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "project_config_poll",
            interval_secs = secs,
            "background project_config materialize poll enabled"
        );
    }

    let app = Router::new()
        .route("/", get(root))
        .route("/docs", get(docs))
        .route("/dos", get(docs))
        .route("/openapi.json", get(openapi))
        .route("/healthz", get(healthz))
        .route("/v1/projects", get(list_projects).post(create_project))
        .route("/v1/projects/{ds_id}", delete(delete_project))
        .route("/v1/projects/{ds_id}/git/push", post(push_project_git))
        .route("/v1/init", post(init_workspace))
        .route("/v1/solve", post(solve))
        .route("/v1/start", post(solve_start))
        .route("/v1/solve_async", post(solve_async))
        .route("/v1/tasks/{task_id}", get(get_task))
        .route("/v1/tasks/{task_id}/cancel", post(cancel_task))
        .route(
            "/v1/sessions/{session_id}/execution",
            get(get_session_execution),
        )
        .route("/v1/biz_advice_report", get(get_biz_advice_report))
        .route(
            "/v1/internal/turns/{turn_id}/assistant-stream",
            post(internal_assistant_stream),
        )
        .route("/v1/biz_advice_report_bak", get(get_biz_advice_report_bak))
        .route(
            "/v1/dev/biz_report_seed_task",
            post(dev_seed_biz_report_task),
        )
        .route(
            "/v1/project/claude/{ds_id}",
            get(get_project_claude_md).post(update_project_claude_md),
        )
        .route("/v1/project/skills/{ds_id}", post(upsert_project_skill))
        .route(
            "/v1/project/prompt/{ds_id}/effective",
            get(get_effective_prompt).post(post_effective_prompt),
        )
        .route(
            "/v1/project/config/{ds_id}",
            get(get_project_config).put(put_project_config),
        )
        .route(
            "/v1/project/config/{ds_id}/versions",
            get(list_project_config_versions),
        )
        .route(
            "/v1/project/config/{ds_id}/versions/compare",
            get(compare_project_config_versions),
        )
        .route(
            "/v1/project/config/{ds_id}/entities/{domain}/{entity_key}/versions/compare",
            get(compare_project_entity_versions),
        )
        .route(
            "/v1/project/config/{ds_id}/entities/{domain}/{entity_key}/versions",
            get(list_project_entity_versions),
        )
        .route(
            "/v1/project/config/{ds_id}/entities/{domain}/{entity_key}/restore",
            post(restore_project_entity_revision),
        )
        .route(
            "/v1/project/config/{ds_id}/versions/commit",
            post(commit_project_config_draft),
        )
        .route(
            "/v1/project/config/{ds_id}/versions/{content_rev}",
            delete(delete_project_config_version).patch(patch_project_config_version_note),
        )
        .route(
            "/v1/project/config/{ds_id}/versions/{content_rev}/activate",
            post(activate_project_config_version),
        )
        .route("/v1/project/tools/catalog", get(get_project_tools_catalog))
        .route(
            "/v1/gateway/global-settings",
            get(get_gateway_global_settings_handler),
        )
        .route(
            "/v1/gateway/global-settings/git-pats",
            post(upsert_gateway_git_pat_handler),
        )
        .route(
            "/v1/gateway/global-settings/git-pats/{pat_id}",
            delete(delete_gateway_git_pat_handler),
        )
        .route("/v1/skills/{ds_id}/{skill_name}", get(get_ds_skill))
        .route("/v1/skills/{ds_id}", get(list_ds_skills))
        .route("/v1/mcp/inject", post(inject_mcp))
        .route("/v1/mcp/injected/{ds_id}", get(get_injected_mcp))
        .route("/v1/mcp/injected/{ds_id}", delete(delete_injected_mcp))
        .route(
            "/v1/agent/feedback",
            post(post_agent_feedback).get(get_agent_feedback),
        )
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
        (
            "POST",
            "/v1/start",
            "Register session in SQLite (sync); returns sessionId / requestId (no solve)",
        ),
        ("POST", "/v1/solve_async", "Create async solve task"),
        ("GET", "/v1/tasks/{task_id}", "Get async task status"),
        (
            "POST",
            "/v1/tasks/{task_id}/cancel",
            "Cancel a queued or running async solve task",
        ),
        (
            "GET",
            "/v1/biz_advice_report?sessionId=…&turnId=…&dsId=…",
            "Report: default LLM polish (biz_advice_report_bak); live spill when CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL=1",
        ),
        (
            "GET",
            "/v1/biz_advice_report_bak?task_id=xx",
            "Legacy LLM-polished report from async task output",
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
        (
            "GET",
            "/v1/project/config/{ds_id}",
            "Get project_config row for ds (PostgreSQL)",
        ),
        (
            "PUT",
            "/v1/project/config/{ds_id}",
            "Upsert project_config for ds (rules / MCP / skills sources / tools / CLAUDE.md)",
        ),
        (
            "GET",
            "/v1/project/tools/catalog",
            "Gateway-registered tool catalog for project selection",
        ),
        (
            "GET",
            "/v1/skills/{ds_id}",
            "List ds workspace skills (home/skills/*/SKILL.md)",
        ),
        (
            "GET",
            "/v1/skills/{ds_id}/{skill_name}",
            "Get one skill file content for ds",
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
                "DsSkillEntry": {
                    "type": "object",
                    "required": ["skill_name", "skill_content"],
                    "properties": {
                        "skill_name": { "type": "string" },
                        "skill_content": { "type": "string" }
                    }
                },
                "DsSkillsListResponse": {
                    "type": "object",
                    "required": ["ds_id", "skills"],
                    "properties": {
                        "ds_id": { "type": "integer", "format": "int64" },
                        "skills": { "type": "array", "items": { "$ref": "#/components/schemas/DsSkillEntry" } }
                    }
                },
                "DsSkillGetResponse": {
                    "type": "object",
                    "required": ["ds_id", "skill_name", "skill_content"],
                    "properties": {
                        "ds_id": { "type": "integer", "format": "int64" },
                        "skill_name": { "type": "string" },
                        "skill_content": { "type": "string" }
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
                "StartRequest": {
                    "type": "object",
                    "required": ["dsId"],
                    "properties": {
                        "dsId": { "type": "integer", "format": "int64", "minimum": 1, "description": "Datasource ID" },
                        "sessionId": { "type": "string", "nullable": true, "description": "Optional: continue an existing gateway session for this dsId (must exist in gateway session DB)." },
                        "extraSession": { "type": "object", "nullable": true, "description": "Optional session-level context stored with the workspace (max 8KB serialized)" }
                    }
                },
                "SolveStartResponse": {
                    "type": "object",
                    "required": ["sessionId", "requestId"],
                    "properties": {
                        "sessionId": { "type": "string", "description": "Gateway session id (registered in SQLite before response)" },
                        "requestId": { "type": "string", "description": "Same value as sessionId for tracing" }
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
                    "required": ["taskId", "sessionId", "requestId", "dsId", "status", "createdAtMs"],
                    "properties": {
                        "taskId": { "type": "string" },
                        "sessionId": { "type": "string" },
                        "requestId": { "type": "string" },
                        "dsId": { "type": "integer", "format": "int64" },
                        "status": { "type": "string" },
                        "createdAtMs": { "type": "integer", "format": "int64" },
                        "startedAtMs": { "type": "integer", "format": "int64", "nullable": true },
                        "finishedAtMs": { "type": "integer", "format": "int64", "nullable": true },
                        "currentTaskDesc": { "type": "string", "nullable": true },
                        "progressUpdatedAtMs": { "type": "integer", "format": "int64", "nullable": true },
                        "hasReport": {
                            "type": "boolean",
                            "description": "True when status is succeeded, or while running once spill/result contains __CLAW_REPORT_START__"
                        },
                        "turnId": { "type": "string" },
                        "progressHistory": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "kind": { "type": "string" },
                                    "message": { "type": "string" },
                                    "tsMs": { "type": "integer", "format": "int64" }
                                }
                            }
                        },
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
            "/v1/start": {
                "post": {
                    "summary": "Register gateway session (sync)",
                    "description": "Synchronously writes (sessionId, dsId) to gateway SQLite, prepares session workspace, and returns sessionId/requestId. Does not run solve; use /v1/solve or /v1/solve_async with the returned sessionId for the first question.",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/StartRequest" } } }
                    },
                    "responses": {
                        "200": { "description": "Session registered", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SolveStartResponse" } } } },
                        "400": { "description": "Unknown sessionId for continuation" }
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
                        "200": { "description": "Task cancelled, or idempotent no-op when already terminal (see TaskRecord.error)", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TaskRecord" } } } },
                        "404": { "description": "Unknown task id" }
                    }
                }
            },
            "/v1/biz_advice_report": {
                "get": {
                    "summary": "Business report: default LLM polish; live spill tail when CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL=1",
                    "parameters": [
                        { "name": "sessionId", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "turnId", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "dsId", "in": "query", "required": true, "schema": { "type": "integer", "format": "int64" } },
                        { "name": "stream", "in": "query", "required": false, "schema": { "type": "boolean", "default": true }, "description": "When true (default): spill SSE if spill file exists, else biz.report.* LLM polish stream from solve output" }
                    ],
                    "responses": {
                        "200": { "description": "Report JSON or SSE", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/BizAdviceReportResponse" } } } }
                    }
                }
            },
            "/v1/biz_advice_report_bak": {
                "get": {
                    "summary": "Legacy: LLM-polished report from async task output (task_id)",
                    "parameters": [
                        { "name": "task_id", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "stream", "in": "query", "required": false, "schema": { "type": "boolean", "default": false }, "description": "When true, response is text/event-stream (biz.report.start / delta / done)" }
                    ],
                    "responses": {
                        "200": { "description": "Polished business advice report (JSON) or SSE when stream=true", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/BizAdviceReportResponse" } } } }
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
            "/v1/skills/{ds_id}": {
                "get": {
                    "summary": "List skills under ds workspace (home/skills/*/SKILL.md)",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } }
                    ],
                    "responses": {
                        "200": { "description": "Skills list", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DsSkillsListResponse" } } } }
                    }
                }
            },
            "/v1/skills/{ds_id}/{skill_name}": {
                "get": {
                    "summary": "Get one skill by name for ds",
                    "parameters": [
                        { "name": "ds_id", "in": "path", "required": true, "schema": { "type": "integer", "format": "int64" } },
                        { "name": "skill_name", "in": "path", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": { "description": "Skill content", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DsSkillGetResponse" } } } },
                        "404": { "description": "Skill not found" }
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

/// Injects `CLAW_PROJECTS_GIT_TOKEN` as `x-access-token:<token>@host` when the URL has no userinfo.
/// Applies to both `https://` (GitHub-style PAT) and `http://` (e.g. internal GitLab). GitLab deploy
/// tokens that need `oauth2:` or `gitlab-ci-token:` can use an explicit userinfo URL instead. kejiqing
fn projects_git_effective_clone_url(url: &str, token: Option<&str>) -> String {
    let base = url.trim();
    if let Some(t) = token.filter(|s| !s.trim().is_empty()) {
        if let Some(rest) = base.strip_prefix("https://") {
            if !rest.contains('@') {
                return format!("https://x-access-token:{t}@{rest}");
            }
        }
        if let Some(rest) = base.strip_prefix("http://") {
            if !rest.contains('@') {
                return format!("http://x-access-token:{t}@{rest}");
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

/// Non-empty CLAUDE.md on one of the project paths. kejiqing
async fn claude_instructions_usable(path: &Path) -> bool {
    let meta = match fs::metadata(path).await {
        Ok(m) if m.is_file() => m,
        _ => return false,
    };
    if meta.len() == 0 {
        return false;
    }
    match fs::read_to_string(path).await {
        Ok(text) => !text.trim().is_empty(),
        Err(_) => false,
    }
}

/// Project tree is ready for solve when CLAUDE instructions exist and are non-empty (pool bind-mount contract). kejiqing
async fn ds_project_tree_ready(work_dir: &Path) -> bool {
    let (home_claude, root_claude) = project_claude_paths(work_dir);
    claude_instructions_usable(&home_claude).await || claude_instructions_usable(&root_claude).await
}

fn ds_environment_not_prepared_error(ds_id: i64, has_project_config: bool) -> ApiError {
    let hint = if has_project_config {
        format!(
            "ds {ds_id} environment not prepared: project_config exists but home/CLAUDE.md is missing or empty; \
             set claudeMd in PUT /v1/project/config/{ds_id}, then POST /v1/init"
        )
    } else {
        format!(
            "ds {ds_id} environment not prepared: no project_config row; \
             POST /v1/projects or PUT /v1/project/config/{ds_id} with non-empty claudeMd, then POST /v1/init"
        )
    };
    ApiError::new(StatusCode::PRECONDITION_FAILED, hint)
}

fn map_project_config_apply_err(e: &project_config_apply::ProjectConfigApplyError) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn write_ds_settings_json(state: &AppState, ds_id: i64) -> Result<(), ApiError> {
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
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
    Ok(())
}

/// Materialize `project_config` from `PostgreSQL` when present (`content_rev` or missing CLAUDE). Author: kejiqing
async fn apply_project_config_for_ds(
    state: &AppState,
    ds_id: i64,
    force: bool,
) -> Result<(), ApiError> {
    apply_project_config_for_ds_inner(state, ds_id, force, true).await
}

async fn apply_project_config_for_ds_inner(
    state: &AppState,
    ds_id: i64,
    force: bool,
    auto_git_push: bool,
) -> Result<(), ApiError> {
    let row = project_config_draft::row_for_materialize(&state.session_db, ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(row) = row else {
        return Ok(());
    };
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create ds work dir failed: {e}"),
            )
        })?;
    let tree_ready = ds_project_tree_ready(&work_dir).await;
    let force_apply = force || !tree_ready;
    let scaffold = gateway_global_settings::load_system_prompt_default(&state.session_db)
        .await
        .map_err(|e| session_db_err(&e))?;
    project_config_apply::apply_if_needed(&work_dir, &row, force_apply, &scaffold)
        .await
        .map_err(|e| map_project_config_apply_err(&e))?;
    write_ds_settings_json(state, ds_id).await?;
    if auto_git_push {
        maybe_push_project_git(state, ds_id).await;
    }
    Ok(())
}

async fn maybe_push_project_git(state: &AppState, ds_id: i64) {
    let Ok(row) = state.session_db.get_project_config(ds_id).await else {
        return;
    };
    let Some(row) = row else {
        return;
    };
    if !parse_git_sync_json(&row.git_sync_json).enabled {
        return;
    }
    if let Err(e) = try_push_project_git(state, ds_id).await {
        warn!(
            target: "claw_gateway_orchestration",
            ds_id,
            error = %e.message,
            "per-project git push after apply failed (non-fatal)"
        );
    }
}

/// One-way push `home/` → per-project remote; updates `git_sync_json` lastPush* in DB. Author: kejiqing
async fn try_push_project_git(state: &AppState, ds_id: i64) -> Result<GitPushOutcome, project_git_sync::ProjectGitSyncError> {
    let row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| project_git_sync::ProjectGitSyncError::new(format!("db: {e}")))?;
    let Some(row) = row else {
        return Err(project_git_sync::ProjectGitSyncError::new(
            "no project_config row",
        ));
    };
    let sync_raw = parse_git_sync_json(&row.git_sync_json);
    if !sync_raw.enabled {
        return Err(project_git_sync::ProjectGitSyncError::new(
            "git sync is disabled",
        ));
    }
    let pat_tokens = gateway_global_settings::load_git_pat_tokens(&state.session_db)
        .await
        .map_err(|e| project_git_sync::ProjectGitSyncError::new(format!("global settings: {e}")))?;
    let sync = project_git_sync::resolve_git_sync_credentials(&sync_raw, &pat_tokens.tokens);
    if let Err(msg) = project_git_sync::validate_git_sync_resolved(&sync) {
        return Err(project_git_sync::ProjectGitSyncError::new(msg));
    }
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    let author = state.cfg.projects_git_author.trim();
    let author = if author.is_empty() {
        "claw-gateway <gateway@claw.local>"
    } else {
        author
    };
    let (author_name, author_email) = parse_projects_git_author(author);
    let excluded = project_config_apply::git_excluded_home_relpaths(&row);
    match project_git_sync::push_home_oneway(
        &work_dir,
        &sync,
        &excluded,
        &author_name,
        &author_email,
    )
    .await
    {
        Ok(outcome) => {
            let mut updated = sync;
            updated.last_push_at_ms = Some(now_ms());
            updated.last_push_commit_id = outcome.commit_id.clone();
            updated.last_push_error = None;
            let git_sync_json = git_sync_to_json(&updated);
            persist_git_sync_status(state, &row, &git_sync_json)
                .await
                .map_err(|e| project_git_sync::ProjectGitSyncError::new(format!("db upsert: {e}")))?;
            Ok(outcome)
        }
        Err(e) => {
            let mut updated = sync;
            updated.last_push_at_ms = Some(now_ms());
            updated.last_push_error = Some(e.message.clone());
            let git_sync_json = git_sync_to_json(&updated);
            let _ = persist_git_sync_status(state, &row, &git_sync_json).await;
            Err(e)
        }
    }
}

async fn persist_git_sync_status(
    state: &AppState,
    row: &session_db::ProjectConfigRow,
    git_sync_json: &Value,
) -> Result<(), sqlx::Error> {
    let mut updated = row.clone();
    updated.git_sync_json = git_sync_json.clone();
    state
        .session_db
        .upsert_project_config(project_config_draft::upsert_from_row(
            &updated,
            &updated.content_rev,
            now_ms(),
            updated.claude_md.as_deref(),
            updated.stable_content_rev.as_deref(),
        ))
        .await
}

async fn sync_ds_project_from_git_mirror(state: &AppState, ds_id: i64) -> Result<(), ApiError> {
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    let _mirror = state.projects_git_mirror_lock.lock().await;
    let repo_dir = projects_git_mirror_pull_impl(&state.cfg.work_root, state.cfg.as_ref()).await?;
    sync_ds_home_from_repo(&repo_dir, &work_dir, ds_id).await
}

/// Before pool acquire: local `ds_<id>` must already have non-empty CLAUDE (provision via `POST /v1/init` or poll). kejiqing
async fn ensure_ds_project_ready(state: &AppState, ds_id: i64) -> Result<(), ApiError> {
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    fs::create_dir_all(work_dir.join(".claw"))
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create ds work dir failed: {e}"),
            )
        })?;
    let has_project_config = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .is_some();
    apply_project_config_for_ds(state, ds_id, false).await?;
    if ds_project_tree_ready(&work_dir).await {
        return Ok(());
    }
    Err(ds_environment_not_prepared_error(ds_id, has_project_config))
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

fn entity_revision_err(e: project_entity_revision::EntityRevisionError) -> ApiError {
    ApiError::new(e.status, e.message)
}

async fn list_project_entity_versions(
    State(state): State<AppState>,
    AxumPath((ds_id, domain, entity_key)): AxumPath<(i64, String, String)>,
) -> Result<Json<project_entity_revision::EntityVersionsListResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    project_entity_revision::list_entity_versions(&state.session_db, ds_id, &domain, &entity_key)
        .await
        .map(Json)
        .map_err(entity_revision_err)
}

#[derive(Debug, Deserialize)]
struct EntityCompareQuery {
    from: String,
    to: String,
}

async fn compare_project_entity_versions(
    State(state): State<AppState>,
    AxumPath((ds_id, domain, entity_key)): AxumPath<(i64, String, String)>,
    Query(q): Query<EntityCompareQuery>,
) -> Result<Json<project_entity_revision::EntityCompareResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    project_entity_revision::compare_entity_versions(
        &state.session_db,
        ds_id,
        &domain,
        &entity_key,
        &q.from,
        &q.to,
    )
    .await
    .map(Json)
    .map_err(entity_revision_err)
}

async fn restore_project_entity_revision(
    State(state): State<AppState>,
    AxumPath((ds_id, domain, entity_key)): AxumPath<(i64, String, String)>,
    Json(req): Json<project_entity_revision::RestoreEntityRevisionRequest>,
) -> Result<Json<project_entity_revision::RestoreEntityRevisionResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    project_entity_revision::restore_entity_revision_to_draft(
        &state.session_db,
        ds_id,
        &domain,
        &entity_key,
        &req.entity_rev,
    )
    .await
    .map(Json)
    .map_err(entity_revision_err)
}

fn draft_err(e: project_config_draft::DraftError) -> ApiError {
    ApiError::new(e.status, e.message)
}

fn default_project_config_row(ds_id: i64) -> session_db::ProjectConfigRow {
    session_db::ProjectConfigRow {
        ds_id,
        content_rev: String::new(),
        stable_content_rev: None,
        draft_open: false,
        updated_at_ms: 0,
        rules_json: json!([]),
        mcp_servers_json: json!({}),
        skills_sources_json: json!([]),
        skills_json: json!([]),
        allowed_tools_json: json!([]),
        claude_md: None,
        git_sync_json: json!({}),
    }
}

fn revision_row_from_upsert<'a>(
    ds_id: i64,
    content_rev: &'a str,
    created_at_ms: i64,
    upsert: &session_db::ProjectConfigUpsert<'a>,
) -> session_db::ProjectConfigRevisionRow {
    session_db::ProjectConfigRevisionRow {
        ds_id,
        content_rev: content_rev.to_string(),
        created_at_ms,
        note: None,
        rules_json: upsert.rules_json.clone(),
        mcp_servers_json: upsert.mcp_servers_json.clone(),
        skills_sources_json: upsert.skills_sources_json.clone(),
        skills_json: upsert.skills_json.clone(),
        allowed_tools_json: upsert.allowed_tools_json.clone(),
        claude_md: upsert.claude_md.map(str::to_string),
    }
}

fn project_config_version_entry_from_summary(
    r: &session_db::ProjectConfigRevisionSummary,
    effective: &str,
) -> ProjectConfigVersionEntry {
    ProjectConfigVersionEntry {
        content_rev: r.content_rev.clone(),
        created_at_ms: r.created_at_ms,
        is_draft: false,
        note: r.note.clone(),
        is_active: r.content_rev == effective,
        claude_in_db: r.claude_in_db,
        skills_count_db: r.skills_count_db,
        rules_count_db: r.rules_count_db,
        mcp_servers_count_db: r.mcp_servers_count_db,
    }
}

fn project_config_version_entry_from_draft(row: &session_db::ProjectConfigRow) -> ProjectConfigVersionEntry {
    let claude_in_db = row
        .claude_md
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());
    let skills_count_db = row.skills_json.as_array().map(|a| a.len() as i64).unwrap_or(0);
    let rules_count_db = row.rules_json.as_array().map(|a| a.len() as i64).unwrap_or(0);
    let mcp_servers_count_db = row
        .mcp_servers_json
        .as_object()
        .map(|o| o.len() as i64)
        .unwrap_or(0);
    ProjectConfigVersionEntry {
        content_rev: project_config_draft::DRAFT_CONTENT_REV.to_string(),
        created_at_ms: row.updated_at_ms,
        is_draft: true,
        note: None,
        is_active: false,
        claude_in_db,
        skills_count_db,
        rules_count_db,
        mcp_servers_count_db,
    }
}

async fn load_revision_for_compare(
    state: &AppState,
    ds_id: i64,
    content_rev: &str,
    active: &session_db::ProjectConfigRow,
) -> Result<session_db::ProjectConfigRevisionRow, ApiError> {
    if project_config_draft::is_draft_content_rev(content_rev) {
        if !active.draft_open {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("no open draft for ds {ds_id}"),
            ));
        }
        return Ok(project_config_draft::revision_row_from_config_row(
            active,
            project_config_draft::DRAFT_CONTENT_REV,
            None,
        ));
    }
    project_config_draft::require_formal_revision(&state.session_db, ds_id, content_rev)
        .await
        .map_err(draft_err)
}

fn revision_row_from_active(row: &session_db::ProjectConfigRow) -> session_db::ProjectConfigRevisionRow {
    session_db::ProjectConfigRevisionRow {
        ds_id: row.ds_id,
        content_rev: row.content_rev.clone(),
        created_at_ms: row.updated_at_ms,
        note: None,
        rules_json: row.rules_json.clone(),
        mcp_servers_json: row.mcp_servers_json.clone(),
        skills_sources_json: row.skills_sources_json.clone(),
        skills_json: row.skills_json.clone(),
        allowed_tools_json: row.allowed_tools_json.clone(),
        claude_md: row.claude_md.clone(),
    }
}

async fn archive_project_config_revision(
    state: &AppState,
    rev: session_db::ProjectConfigRevisionRow,
) -> Result<(), ApiError> {
    if project_config_draft::is_draft_content_rev(&rev.content_rev) {
        return Ok(());
    }
    let inserted = state
        .session_db
        .insert_project_config_revision_immutable(&rev)
        .await
        .map_err(|e| session_db_err(&e))?;
    if !inserted {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "revision {} already exists and cannot be changed",
                rev.content_rev
            ),
        ));
    }
    Ok(())
}

async fn activate_project_config_revision_row(
    state: &AppState,
    ds_id: i64,
    rev: session_db::ProjectConfigRevisionRow,
    git_sync_json: Value,
) -> Result<bool, ApiError> {
    let now = now_ms();
    state
        .session_db
        .upsert_project_config(session_db::ProjectConfigUpsert {
            ds_id,
            content_rev: &rev.content_rev,
            stable_content_rev: Some(rev.content_rev.as_str()),
            draft_open: false,
            updated_at_ms: now,
            rules_json: &rev.rules_json,
            mcp_servers_json: &rev.mcp_servers_json,
            skills_sources_json: &rev.skills_sources_json,
            skills_json: &rev.skills_json,
            allowed_tools_json: &rev.allowed_tools_json,
            claude_md: rev.claude_md.as_deref(),
            git_sync_json: &git_sync_json,
        })
        .await
        .map_err(|e| session_db_err(&e))?;
    let lock = get_ds_lock(state, ds_id).await;
    let _guard = lock.lock().await;
    apply_project_config_for_ds_inner(state, ds_id, true, true).await?;
    let applied = project_config_apply::read_applied_content_rev(&ds_work_dir(
        &state.cfg.work_root,
        ds_id,
    ))
    .await;
    Ok(applied.as_deref() == Some(rev.content_rev.as_str()))
}

fn merge_git_sync_from_put(incoming: &Value, existing: &Value) -> Value {
    let mut inc = parse_git_sync_json(incoming);
    let ex = parse_git_sync_json(existing);
    let pat_id_in_incoming = incoming.get("gitPatId").is_some();
    if !pat_id_in_incoming {
        inc.git_pat_id = ex.git_pat_id;
    } else if incoming.get("gitPatId").map_or(false, |v| v.is_null()) {
        inc.git_pat_id = None;
    }
    let uses_global_pat = inc
        .git_pat_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    if uses_global_pat {
        inc.git_token = None;
    } else if inc
        .git_token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none()
    {
        inc.git_token = ex.git_token;
    }
    git_sync_to_json(&inc)
}

async fn git_sync_json_for_api(state: &AppState, v: &Value) -> Value {
    let sync = parse_git_sync_json(v);
    let tokens = gateway_global_settings::load_git_pat_tokens(&state.session_db)
        .await
        .ok();
    let token_set = git_sync_token_set(&sync, tokens.as_ref());
    let mut j = git_sync_to_json(&sync);
    if let Some(obj) = j.as_object_mut() {
        obj.insert("gitTokenSet".into(), json!(token_set));
    }
    j
}

fn git_sync_token_set(
    sync: &project_git_sync::ProjectGitSync,
    tokens: Option<&gateway_global_settings::GitPatTokensStore>,
) -> bool {
    if sync
        .git_token
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    let Some(id) = sync.git_pat_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    tokens
        .map(|t| t.tokens.contains_key(id))
        .unwrap_or(false)
}

async fn load_project_config_or_default(
    state: &AppState,
    ds_id: i64,
) -> Result<session_db::ProjectConfigRow, ApiError> {
    Ok(state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .unwrap_or_else(|| default_project_config_row(ds_id)))
}

fn merge_skill_into_skills_json(skills_json: &mut Value, skill_name: &str, skill_content: &str) {
    if !skills_json.is_array() {
        *skills_json = json!([]);
    }
    let arr = skills_json.as_array_mut().expect("skills_json is array");
    for item in arr.iter_mut() {
        if item.get("skillName").and_then(Value::as_str) == Some(skill_name) {
            if let Some(obj) = item.as_object_mut() {
                obj.insert("skillContent".into(), json!(skill_content));
            }
            return;
        }
    }
    arr.push(json!({
        "skillName": skill_name,
        "skillContent": skill_content,
    }));
}

fn validate_skills_json(skills: &Value) -> Result<(), ApiError> {
    let arr = skills.as_array().ok_or_else(|| {
        ApiError::new(StatusCode::BAD_REQUEST, "skillsJson must be a JSON array")
    })?;
    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("skillsJson[{i}] must be a JSON object"),
            )
        })?;
        let name = obj
            .get("skillName")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!("skillsJson[{i}] missing skillName"),
                )
            })?;
        validate_skill_name(name)?;
        if !obj.contains_key("skillContent") {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("skillsJson[{i}] missing skillContent"),
            ));
        }
    }
    Ok(())
}

fn reject_deprecated_skills_sources(sources: &Value) -> Result<(), ApiError> {
    if sources.as_array().is_some_and(|a| !a.is_empty()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "skillsSourcesJson is deprecated; use skillsJson (inline skills stored in project_config)",
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

async fn run_git(cwd: &Path, args: &[&str]) -> Result<String, ApiError> {
    run_git_env(cwd, &[], args).await
}

/// Bind-mounted `.claw-code-projects` is often owned by the host user; gateway runs as another uid. kejiqing
async fn ensure_projects_git_safe_directory(work_root: &Path) {
    let repo_dir = projects_repo_dir(work_root);
    let path = repo_dir.display().to_string();
    if let Err(e) = run_git(
        work_root,
        &["config", "--global", "--add", "safe.directory", &path],
    )
    .await
    {
        warn!(
            target: "claw_gateway_orchestration",
            component = "projects_git",
            phase = "safe_directory",
            repo_dir = %repo_dir.display(),
            error = %e.detail(),
            "git safe.directory failed; mirror pull/init may fail with dubious ownership"
        );
    }
}

/// Best-effort `git rev-parse` for health/diagnostics (no pull). kejiqing
async fn git_rev_parse_optional(cwd: &Path, spec: &str) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    cmd.args(["rev-parse", spec]);
    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        None
    } else {
        Some(stdout)
    }
}

async fn count_skill_dirs(skills_root: &Path) -> u64 {
    let mut rd = match fs::read_dir(skills_root).await {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut n = 0u64;
    while let Ok(Some(ent)) = rd.next_entry().await {
        if ent.file_type().await.is_ok_and(|t| t.is_dir()) {
            n += 1;
        }
    }
    n
}

/// Read-only snapshot of per-`ds_*` workspace readiness (for `/healthz`). Author: kejiqing
async fn build_ds_workspaces_health(work_root: &Path) -> Value {
    let on_disk = list_ds_ids_under_work_root(work_root)
        .await
        .unwrap_or_default();
    let ids = on_disk;

    let mut workspaces = Vec::new();
    let mut prepared_count = 0u64;
    for ds_id in ids {
        let work_dir = ds_work_dir(work_root, ds_id);
        let work_dir_present = fs::metadata(&work_dir).await.is_ok_and(|m| m.is_dir());
        let environment_prepared = work_dir_present && ds_project_tree_ready(&work_dir).await;
        if environment_prepared {
            prepared_count += 1;
        }
        let (home_claude, root_claude) = project_claude_paths(&work_dir);
        let claude_home_present = fs::metadata(&home_claude).await.is_ok_and(|m| m.is_file());
        let claude_root_present = fs::metadata(&root_claude).await.is_ok_and(|m| m.is_file());
        let claude_home_bytes = if claude_home_present {
            fs::metadata(&home_claude).await.ok().map(|m| m.len())
        } else {
            None
        };
        let skills_root = work_dir.join("home/skills");
        let skills_count = if fs::metadata(&skills_root).await.is_ok_and(|m| m.is_dir()) {
            count_skill_dirs(&skills_root).await
        } else {
            0
        };

        workspaces.push(json!({
            "dsId": ds_id,
            "workDir": work_dir.display().to_string(),
            "workDirPresent": work_dir_present,
            "environmentPrepared": environment_prepared,
            "claudeHomePath": home_claude.display().to_string(),
            "claudeHomePresent": claude_home_present,
            "claudeHomeBytes": claude_home_bytes,
            "claudeRootPresent": claude_root_present,
            "skillsCount": skills_count,
        }));
    }

    json!({
        "dsWorkspaceCount": workspaces.len(),
        "environmentPreparedCount": prepared_count,
        "dsWorkspaces": workspaces,
    })
}

/// All subprocess git calls use HTTP/1.1 for libcurl remotes to reduce HTTP/2 framing / packfile
/// failures on some networks (common with long-haul links). Local-only commands ignore this. kejiqing
async fn run_git_env(
    cwd: &Path,
    env_pairs: &[(&str, &str)],
    args: &[&str],
) -> Result<String, ApiError> {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    cmd.args(["-c", "http.version=HTTP/1.1"]);
    for (k, v) in env_pairs {
        cmd.env(k, v);
    }
    cmd.args(args);
    let output = cmd
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
        let shown = if args.is_empty() {
            "-c http.version=HTTP/1.1".to_string()
        } else {
            format!("-c http.version=HTTP/1.1 {}", args.join(" "))
        };
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("git {shown} failed: {detail}"),
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
    ensure_projects_git_safe_directory(work_root).await;
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

/// Apply all `project_config` rows to disk before HTTP listen (and on each poll tick). Author: kejiqing
async fn run_startup_project_config_apply(state: &AppState) {
    info!(
        target: "claw_gateway_orchestration",
        component = "startup",
        phase = "project_config_startup_apply",
        "materializing project_config rows to ds workspaces before accepting traffic"
    );
    match tick_project_config_apply_poll(state).await {
        Ok(()) => info!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "project_config_startup_apply",
            "startup project_config apply completed"
        ),
        Err(e) => warn!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "project_config_startup_apply",
            status = %e.status,
            error = %e.detail(),
            "startup project_config apply failed; gateway will still listen"
        ),
    }
}

async fn project_config_poll_loop(state: AppState, interval_secs: u64) {
    let start = tokio::time::Instant::now() + Duration::from_secs(interval_secs);
    let mut ticker = tokio::time::interval_at(start, Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        match tick_project_config_apply_poll(&state).await {
            Ok(()) => {}
            Err(e) => {
                warn!(
                    target: "claw_gateway_orchestration",
                    component = "project_config_poll",
                    phase = "tick_failed",
                    status = %e.status,
                    error = %e.detail(),
                    "periodic project_config materialize failed"
                );
            }
        }
    }
}

async fn list_ds_ids_in_projects_mirror(repo_dir: &Path) -> Result<Vec<i64>, ApiError> {
    let mut out = Vec::new();
    let mut rd = fs::read_dir(repo_dir).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("list projects mirror failed: {e}"),
        )
    })?;
    while let Some(ent) = rd.next_entry().await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("read projects mirror entry failed: {e}"),
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

fn merge_sorted_ds_ids(mut a: Vec<i64>, b: Vec<i64>) -> Vec<i64> {
    a.extend(b);
    a.sort_unstable();
    a.dedup();
    a
}

async fn tick_project_config_apply_poll(state: &AppState) -> Result<(), ApiError> {
    let on_disk = list_ds_ids_under_work_root(&state.cfg.work_root).await?;
    let in_config = state
        .session_db
        .list_project_config_ds_ids()
        .await
        .map_err(|e| session_db_err(&e))?;
    let ids = merge_sorted_ds_ids(on_disk, in_config);
    for ds_id in ids {
        let lock = get_ds_lock(state, ds_id).await;
        let Ok(_guard) = lock.try_lock() else {
            continue;
        };
        let cfg_row = state
            .session_db
            .get_project_config(ds_id)
            .await
            .map_err(|e| session_db_err(&e))?;
        let Some(row) = cfg_row else {
            continue;
        };
        let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
        fs::create_dir_all(work_dir.join(".claw"))
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("create ds work dir failed: {e}"),
                )
            })?;
        let applied = project_config_apply::read_applied_content_rev(&work_dir).await;
        if applied.as_deref() != Some(row.content_rev.as_str()) {
            apply_project_config_for_ds(state, ds_id, false).await?;
        }
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

async fn healthz(State(state): State<AppState>, headers: HeaderMap) -> Json<Value> {
    let isolation = state.cfg.solve_isolation.as_str();
    let ds_workspaces = build_ds_workspaces_health(&state.cfg.work_root).await;
    let request_host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok());
    let deploy_image_ref = http_gateway_rs::deploy_image::image_ref_from_env();
    let deploy_image_tag = http_gateway_rs::deploy_image::deploy_image_tag(&deploy_image_ref);
    Json(json!({
        "ok": true,
        "deployImageRef": deploy_image_ref,
        "deployImageTag": deploy_image_tag,
        "clawBin": state.cfg.claw_bin,
        "workRoot": state.cfg.work_root.display().to_string(),
        "registryPath": state.cfg.ds_registry_path.display().to_string(),
        "defaultTimeoutSeconds": state.cfg.default_timeout_seconds,
        "defaultMaxIterations": state.cfg.default_max_iterations,
        "defaultHttpMcpName": state.cfg.default_http_mcp_name,
        "defaultHttpMcpUrl": state.cfg.default_http_mcp_url,
        "defaultHttpMcpTransport": state.cfg.default_http_mcp_transport,
        "solveIsolation": isolation,
        "containerPool": true,
        "poolRpcRemote": state.cfg.pool_rpc_remote,
        "poolRpcTcp": state.cfg.pool_rpc_tcp,
        "poolRpcUnixSocket": state.cfg.pool_rpc_unix_socket,
        "poolRpcHostWorkRoot": state.cfg.pool_rpc_host_work_root.as_ref().map(|p| p.display().to_string()),
        "sessionDatabaseBackend": "postgresql",
        "gatewayDatabaseUrl": state.session_db.database_url_redacted(),
        "projectsGitUrl": state.cfg.projects_git_url.clone(),
        "projectsGitBranch": state.cfg.projects_git_branch.clone(),
        "projectsGitDsHomePollIntervalSecs": state.cfg.projects_git_ds_home_poll_interval_secs,
        "projectsGitMirror": ds_workspaces,
        "reportPolishUsesDeepseek": state.cfg.report_polish_deepseek.is_some(),
        "reportDeepseekModel": state.cfg.report_polish_deepseek.as_ref().map(|d| d.model.clone()),
        "liveBizReportSpillEnabled": state.cfg.live_biz_report_spill_enabled,
        "claudeTap": http_gateway_rs::claude_tap_health::claude_tap_health_json(request_host),
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
    let new_turn_id = turn_id::mint_turn_id();
    register_solve_turn(
        &state.session_db,
        &new_turn_id,
        &effective,
        req.ds_id,
        &req.user_prompt,
    )
    .await?;
    let result = run_solve_request(
        state.clone(),
        req,
        RunSolveContext {
            request_id: effective.clone(),
            task_id: None,
            turn_id: new_turn_id.clone(),
            skip_session_db: false,
        },
    )
    .await;
    match &result {
        Ok(success) => {
            finalize_solve_turn_success(Arc::clone(&state.session_db), &new_turn_id, success).await;
        }
        Err(err) => {
            finalize_solve_turn_failed(&state.session_db, &new_turn_id, err).await;
        }
    }
    let result = result?;
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

fn default_project_claude_md(ds_id: i64) -> String {
    format!(
        "# ds_{ds_id}\n\nAuthor: kejiqing\n\nEdit in admin **CLAUDE.md** or `PUT /v1/project/config/{ds_id}`.\n"
    )
}

async fn collect_known_ds_ids(state: &AppState) -> Result<Vec<i64>, ApiError> {
    let on_disk = list_ds_ids_under_work_root(&state.cfg.work_root).await?;
    let in_config = state
        .session_db
        .list_project_config_ds_ids()
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(merge_sorted_ds_ids(on_disk, in_config))
}

async fn resolve_create_ds_id(state: &AppState, requested: Option<i64>) -> Result<i64, ApiError> {
    if let Some(id) = requested {
        if id < 1 {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
        }
        return Ok(id);
    }
    let ids = collect_known_ds_ids(state).await?;
    Ok(ids.last().copied().unwrap_or(0) + 1)
}

async fn ds_exists_on_stack(state: &AppState, ds_id: i64) -> Result<bool, ApiError> {
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    if fs::metadata(&work_dir).await.is_ok_and(|m| m.is_dir()) {
        return Ok(true);
    }
    Ok(state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .is_some())
}

async fn scaffold_ds_workspace(work_dir: &Path, ds_id: i64) -> Result<(), ApiError> {
    let claude = default_project_claude_md(ds_id);
    fs::create_dir_all(work_dir.join(".claw")).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create .claw failed: {e}"),
        )
    })?;
    fs::create_dir_all(work_dir.join("home/skills")).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create home/skills failed: {e}"),
        )
    })?;
    fs::write(work_dir.join("home/CLAUDE.md"), &claude)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write home/CLAUDE.md failed: {e}"),
            )
        })?;
    fs::write(work_dir.join("CLAUDE.md"), &claude).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write CLAUDE.md failed: {e}"),
        )
    })?;
    Ok(())
}

/// Commit staged changes under `pathspec` and push (shared by project create/delete). Author: kejiqing
async fn projects_git_commit_and_push(
    cfg: &GatewayConfig,
    repo_dir: &Path,
    pathspec: &str,
    commit_message: &str,
) -> Result<GitSyncResponse, ApiError> {
    let dirty = run_git(repo_dir, &["status", "--porcelain", "--", pathspec]).await?;
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
                Err(e) => return Err(e),
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

async fn projects_git_push_ds_home_from_workdir(
    cfg: &GatewayConfig,
    work_root: &Path,
    repo_dir: &Path,
    ds_id: i64,
    commit_message: &str,
) -> Result<GitSyncResponse, ApiError> {
    let work_dir = ds_work_dir(work_root, ds_id);
    let ds_root_in_repo = repo_dir.join(format!("ds_{ds_id}"));
    let dst_home = ds_root_in_repo.join("home");
    if fs::metadata(&dst_home).await.is_ok_and(|m| m.is_dir()) {
        fs::remove_dir_all(&dst_home).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("cleanup repo ds home failed: {e}"),
            )
        })?;
    }
    copy_tree(&work_dir.join("home"), &dst_home).await?;
    let rel_prefix = format!("ds_{ds_id}/");
    run_git(repo_dir, &["add", &rel_prefix]).await?;
    projects_git_commit_and_push(cfg, repo_dir, &rel_prefix, commit_message).await
}

async fn projects_git_remove_ds_tree(
    cfg: &GatewayConfig,
    repo_dir: &Path,
    ds_id: i64,
) -> Result<Option<GitSyncResponse>, ApiError> {
    let rel = format!("ds_{ds_id}");
    if !fs::metadata(repo_dir.join(&rel))
        .await
        .is_ok_and(|m| m.is_dir())
    {
        return Ok(None);
    }
    run_git(repo_dir, &["rm", "-rf", "--ignore-unmatch", &rel]).await?;
    let dirty = run_git(repo_dir, &["status", "--porcelain", "--", &rel]).await?;
    if dirty.trim().is_empty() {
        return Ok(None);
    }
    let msg = format!("chore(projects): remove {rel}");
    Ok(Some(
        projects_git_commit_and_push(cfg, repo_dir, &rel, &msg).await?,
    ))
}

async fn list_projects(State(state): State<AppState>) -> Result<Json<ProjectListResponse>, ApiError> {
    let summaries = state
        .session_db
        .list_project_config_summaries()
        .await
        .map_err(|e| session_db_err(&e))?;
    let mut projects = Vec::with_capacity(summaries.len());
    for s in summaries {
        let work_dir = ds_work_dir(&state.cfg.work_root, s.ds_id);
        let work_dir_present = fs::metadata(&work_dir).await.is_ok_and(|m| m.is_dir());
        let environment_prepared =
            work_dir_present && ds_project_tree_ready(&work_dir).await;
        let (home_claude, _) = project_claude_paths(&work_dir);
        let claude_on_disk = claude_instructions_usable(&home_claude).await;
        let skills_root = work_dir.join("home/skills");
        let skills_count_disk = if fs::metadata(&skills_root).await.is_ok_and(|m| m.is_dir()) {
            count_skill_dirs(&skills_root).await
        } else {
            0
        };
        let applied_rev = project_config_apply::read_applied_content_rev(&work_dir).await;
        let stable_rev = s
            .stable_content_rev
            .as_deref()
            .filter(|r| !project_config_draft::is_draft_content_rev(r))
            .unwrap_or(s.content_rev.as_str());
        let db_synced_to_disk = applied_rev.as_deref() == Some(stable_rev);
        projects.push(ProjectListEntry {
            ds_id: s.ds_id,
            content_rev: stable_rev.to_string(),
            draft_open: s.draft_open,
            updated_at_ms: s.updated_at_ms,
            skills_count_db: s.skills_count_db,
            claude_in_db: s.claude_in_db,
            rules_count_db: s.rules_count_db,
            mcp_servers_count_db: s.mcp_servers_count_db,
            work_dir_present,
            environment_prepared,
            claude_on_disk,
            skills_count_disk,
            applied_rev,
            db_synced_to_disk,
            git_sync: git_sync_list_summary(&s.git_sync_json),
        });
    }
    Ok(Json(ProjectListResponse {
        projects,
        listed_at_ms: now_ms(),
    }))
}

async fn push_project_git(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<ProjectGitPushResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let lock = get_ds_lock(&state, ds_id).await;
    let _guard = lock.lock().await;
    apply_project_config_for_ds_inner(&state, ds_id, false, false).await?;
    let outcome = try_push_project_git(&state, ds_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.message))?;
    let row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists");
    Ok(Json(ProjectGitPushResponse {
        ds_id,
        outcome,
        git_sync_json: git_sync_json_for_api(&state, &row.git_sync_json).await,
    }))
}

async fn create_project(
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<Json<InitResponse>, ApiError> {
    let ds_id = resolve_create_ds_id(&state, req.ds_id).await?;
    if ds_exists_on_stack(&state, ds_id).await? {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("ds {ds_id} already exists (work_root or project_config)"),
        ));
    }
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    let lock = get_ds_lock(&state, ds_id).await;
    let _guard = lock.lock().await;
    scaffold_ds_workspace(&work_dir, ds_id).await?;
    let now = now_ms();
    let content_rev = project_config_draft::format_formal_content_rev_local_ms(now);
    let claude_md = default_project_claude_md(ds_id);
    let empty_obj = json!({});
    let empty_arr = json!([]);
    state
        .session_db
        .upsert_project_config(session_db::ProjectConfigUpsert {
            ds_id,
            content_rev: &content_rev,
            stable_content_rev: Some(content_rev.as_str()),
            draft_open: false,
            updated_at_ms: now,
            rules_json: &empty_arr,
            mcp_servers_json: &empty_obj,
            skills_sources_json: &empty_arr,
            skills_json: &empty_arr,
            allowed_tools_json: &empty_arr,
            claude_md: Some(&claude_md),
            git_sync_json: &json!({}),
        })
        .await
        .map_err(|e| session_db_err(&e))?;
    if let Ok(Some(row)) = state.session_db.get_project_config(ds_id).await {
        archive_project_config_revision(&state, revision_row_from_active(&row)).await?;
    }
    apply_project_config_for_ds(&state, ds_id, true).await?;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    write_ds_settings_json(&state, ds_id).await?;
    Ok(Json(InitResponse {
        ds_id,
        work_dir: work_dir.display().to_string(),
        initialized: true,
    }))
}

async fn delete_project(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Query(query): Query<DeleteProjectQuery>,
) -> Result<Json<DeleteProjectResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    if !ds_exists_on_stack(&state, ds_id).await? {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("ds {ds_id} not found"),
        ));
    }
    let lock = get_ds_lock(&state, ds_id).await;
    let _guard = lock.lock().await;
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    if fs::metadata(&work_dir).await.is_ok_and(|m| m.is_dir()) {
        fs::remove_dir_all(&work_dir).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("remove work_dir failed: {e}"),
            )
        })?;
    }
    let project_config_removed = state
        .session_db
        .delete_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let sessions_removed = if query.purge_sessions {
        state
            .session_db
            .delete_sessions_for_ds(ds_id)
            .await
            .map_err(|e| session_db_err(&e))?
    } else {
        0
    };
    {
        let mut injected = state.injected_mcp.lock().await;
        injected.remove(&ds_id);
    }
    Ok(Json(DeleteProjectResponse {
        ds_id,
        deleted: true,
        purge_sessions: query.purge_sessions,
        sessions_removed,
        project_config_removed,
        git_sync: None,
    }))
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
        let lock = get_ds_lock(&state, req.ds_id).await;
        let _guard = lock.lock().await;
        let has_project_config = state
            .session_db
            .get_project_config(req.ds_id)
            .await
            .map_err(|e| session_db_err(&e))?
            .is_some();
        if !has_project_config {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!(
                    "no project_config for ds {}; POST /v1/projects or PUT /v1/project/config/{} first",
                    req.ds_id, req.ds_id
                ),
            ));
        }
        apply_project_config_for_ds(&state, req.ds_id, true).await?;
        if !ds_project_tree_ready(&work_dir).await {
            return Err(ds_environment_not_prepared_error(req.ds_id, true));
        }
        ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
        write_ds_settings_json(&state, req.ds_id).await?;
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

async fn project_config_row_to_response(
    state: &AppState,
    row: session_db::ProjectConfigRow,
) -> ProjectConfigResponse {
    ProjectConfigResponse {
        ds_id: row.ds_id,
        content_rev: row.content_rev.clone(),
        stable_content_rev: row.stable_content_rev.clone(),
        draft_open: row.draft_open,
        updated_at_ms: row.updated_at_ms,
        rules_json: row.rules_json,
        mcp_servers_json: row.mcp_servers_json,
        skills_sources_json: row.skills_sources_json,
        skills_json: row.skills_json,
        allowed_tools_json: row.allowed_tools_json,
        claude_md: row.claude_md,
        git_sync_json: git_sync_json_for_api(state, &row.git_sync_json).await,
    }
}

async fn project_selected_allowed_tools(
    state: &AppState,
    ds_id: i64,
) -> Result<Option<Vec<String>>, ApiError> {
    let row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let selected = project_tools::parse_allowed_tools_json(&row.allowed_tools_json)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if selected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selected))
    }
}

async fn get_project_tools_catalog(
    State(_state): State<AppState>,
) -> Json<ProjectToolsCatalogResponse> {
    Json(ProjectToolsCatalogResponse {
        tools: project_tools::gateway_registered_tool_catalog(),
    })
}

const SKILLS_SOURCES_FORBIDDEN_CRED_KEYS: &[&str] = &[
    "token",
    "gitToken",
    "accessToken",
    "password",
    "secret",
    "pat",
];

/// Git credentials for `project_config` skills sources: env only (`tokenEnv`), never in JSON/DB.
fn validate_skills_sources_json(sources: &Value) -> Result<(), ApiError> {
    let arr = sources.as_array().ok_or_else(|| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "skillsSourcesJson must be a JSON array",
        )
    })?;
    for (i, item) in arr.iter().enumerate() {
        let obj = item.as_object().ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("skillsSourcesJson[{i}] must be a JSON object"),
            )
        })?;
        for key in SKILLS_SOURCES_FORBIDDEN_CRED_KEYS {
            if obj.contains_key(*key) {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "skillsSourcesJson[{i}]: git credentials must not be stored in project_config; use tokenEnv pointing to a gateway environment variable"
                    ),
                ));
            }
        }
        let Some(git_url) = obj
            .get("gitUrl")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let is_http = git_url.starts_with("https://") || git_url.starts_with("http://");
        if is_http && git_url.contains('@') {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "skillsSourcesJson[{i}]: gitUrl must not embed userinfo; set tokenEnv to an env var name (git token is env-only)"
                ),
            ));
        }
        if is_http {
            let token_env = obj
                .get("tokenEnv")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(token_env) = token_env else {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "skillsSourcesJson[{i}]: tokenEnv is required for HTTP(S) gitUrl without embedded credentials"
                    ),
                ));
            };
            if !token_env
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "skillsSourcesJson[{i}]: tokenEnv must be an ASCII env var name [A-Za-z0-9_]"
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_project_config_payload(req: &UpsertProjectConfigRequest) -> Result<(), ApiError> {
    if !req.rules_json.is_array() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "rulesJson must be a JSON array",
        ));
    }
    if !req.mcp_servers_json.is_object() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "mcpServersJson must be a JSON object",
        ));
    }
    if !req.allowed_tools_json.is_array() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "allowedToolsJson must be a JSON array",
        ));
    }
    reject_deprecated_skills_sources(&req.skills_sources_json)?;
    validate_skills_json(&req.skills_json)?;
    project_tools::validate_project_allowed_tools_json(&req.allowed_tools_json)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(())
}

async fn get_project_config(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<ProjectConfigResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(row) = row else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for ds {ds_id}"),
        ));
    };
    Ok(Json(project_config_row_to_response(&state, row).await))
}

#[derive(Debug, Serialize)]
struct PutProjectConfigResponse {
    #[serde(rename = "draftOpen")]
    draft_open: bool,
    #[serde(rename = "stableContentRev", skip_serializing_if = "Option::is_none")]
    stable_content_rev: Option<String>,
    #[serde(rename = "activeConfig")]
    active_config: ProjectConfigResponse,
}

#[derive(Debug, Serialize)]
struct CommitProjectConfigDraftResponse {
    #[serde(rename = "savedContentRev")]
    saved_content_rev: String,
    activated: bool,
    #[serde(rename = "stableContentRev")]
    stable_content_rev: String,
    materialized: bool,
    #[serde(rename = "activeConfig")]
    active_config: ProjectConfigResponse,
}

async fn list_project_config_versions(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<ProjectConfigVersionsResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let active = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(active) = active else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for ds {ds_id}"),
        ));
    };
    let revisions = state
        .session_db
        .list_project_config_revisions(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let work_dir = ds_work_dir(&state.cfg.work_root, ds_id);
    let applied_content_rev = project_config_apply::read_applied_content_rev(&work_dir).await;
    let effective = project_config_draft::effective_formal_rev(&active)
        .map_err(draft_err)?
        .to_string();
    project_config_draft::ensure_formal_revision_recorded(
        &state.session_db,
        ds_id,
        &effective,
        &active,
    )
    .await
    .map_err(draft_err)?;
    let mut versions: Vec<ProjectConfigVersionEntry> = revisions
        .into_iter()
        .filter(|r| !project_config_draft::is_draft_content_rev(&r.content_rev))
        .map(|r| project_config_version_entry_from_summary(&r, &effective))
        .collect();
    if active.draft_open {
        versions.insert(0, project_config_version_entry_from_draft(&active));
    }
    Ok(Json(ProjectConfigVersionsResponse {
        ds_id,
        active_content_rev: effective,
        applied_content_rev,
        draft_open: active.draft_open,
        versions,
    }))
}

async fn compare_project_config_versions(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Query(query): Query<CompareProjectConfigQuery>,
) -> Result<Json<project_config_version::ProjectConfigCompareResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let from = query.from.trim();
    let to = query.to.trim();
    if from.is_empty() || to.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "query params from and to are required",
        ));
    }
    let active = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(active) = active else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for ds {ds_id}"),
        ));
    };
    let from_row = load_revision_for_compare(&state, ds_id, from, &active).await?;
    let to_row = load_revision_for_compare(&state, ds_id, to, &active).await?;
    Ok(Json(project_config_version::compare_revision_rows(
        ds_id,
        project_config_draft::effective_formal_rev(&active).map_err(draft_err)?,
        &from_row,
        &to_row,
    )))
}

async fn activate_project_config_version(
    State(state): State<AppState>,
    AxumPath((ds_id, content_rev)): AxumPath<(i64, String)>,
) -> Result<Json<ActivateProjectConfigVersionResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let content_rev = content_rev.trim();
    if content_rev.is_empty() || project_config_draft::is_draft_content_rev(content_rev) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "contentRev must be a saved (non-draft) version id",
        ));
    }
    let active_row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(active_row) = active_row else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for ds {ds_id}"),
        ));
    };
    let rev = project_config_draft::require_formal_revision(&state.session_db, ds_id, content_rev)
        .await
        .map_err(draft_err)?;
    let materialized = activate_project_config_revision_row(
        &state,
        ds_id,
        rev,
        active_row.git_sync_json.clone(),
    )
    .await?;
    Ok(Json(ActivateProjectConfigVersionResponse {
        ds_id,
        active_content_rev: content_rev.to_string(),
        activated: true,
        materialized,
    }))
}

async fn put_project_config(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Json(req): Json<UpsertProjectConfigRequest>,
) -> Result<Json<PutProjectConfigResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let existing = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(existing) = existing else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for ds {ds_id}; create project first"),
        ));
    };
    let existing_git = existing.git_sync_json.clone();
    let git_sync_json = match &req.git_sync_json {
        Some(incoming) => merge_git_sync_from_put(incoming, &existing_git),
        None => existing_git,
    };
    let req_for_validate = UpsertProjectConfigRequest {
        content_rev: String::new(),
        rules_json: req.rules_json.clone(),
        mcp_servers_json: req.mcp_servers_json.clone(),
        skills_sources_json: req.skills_sources_json.clone(),
        skills_json: req.skills_json.clone(),
        allowed_tools_json: req.allowed_tools_json.clone(),
        claude_md: req.claude_md.clone(),
        git_sync_json: Some(git_sync_json.clone()),
    };
    validate_project_config_payload(&req_for_validate)?;
    gateway_global_settings::validate_git_sync_json_with_global(&state.session_db, &git_sync_json)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    project_config_draft::ensure_draft(&state.session_db, ds_id)
        .await
        .map_err(draft_err)?;
    let effective = project_config_draft::effective_formal_rev(&existing)
        .map_err(draft_err)?
        .to_string();
    let now = now_ms();
    let upsert = session_db::ProjectConfigUpsert {
        ds_id,
        content_rev: project_config_draft::DRAFT_CONTENT_REV,
        stable_content_rev: Some(effective.as_str()),
        draft_open: true,
        updated_at_ms: now,
        rules_json: &req.rules_json,
        mcp_servers_json: &req.mcp_servers_json,
        skills_sources_json: &req.skills_sources_json,
        skills_json: &req.skills_json,
        allowed_tools_json: &req.allowed_tools_json,
        claude_md: req.claude_md.as_deref(),
        git_sync_json: &git_sync_json,
    };
    state
        .session_db
        .upsert_project_config(upsert)
        .await
        .map_err(|e| session_db_err(&e))?;
    project_entity_revision::record_draft_put_sidecars(
        &state.session_db,
        ds_id,
        &existing,
        &req.rules_json,
        &req.skills_json,
        &req.mcp_servers_json,
        req.claude_md.as_deref(),
        &req.allowed_tools_json,
        now,
    )
    .await
    .map_err(entity_revision_err)?;
    let active = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists after upsert");
    Ok(Json(PutProjectConfigResponse {
        draft_open: true,
        stable_content_rev: active.stable_content_rev.clone(),
        active_config: project_config_row_to_response(&state, active).await,
    }))
}

async fn commit_project_config_draft(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
    Json(req): Json<CommitProjectConfigDraftRequest>,
) -> Result<Json<CommitProjectConfigDraftResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let note = project_config_draft::normalize_revision_note(req.note);
    let row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("no project_config for ds {ds_id}"),
            )
        })?;
    if !row.draft_open {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "no open draft to commit; edit config first",
        ));
    }
    let git_sync_json = row.git_sync_json.clone();
    let prev_stable = project_config_draft::effective_formal_rev(&row)
        .map_err(draft_err)?
        .to_string();
    project_config_draft::ensure_formal_revision_recorded(
        &state.session_db,
        ds_id,
        &prev_stable,
        &row,
    )
    .await
    .map_err(draft_err)?;
    let now = now_ms();
    let saved = project_config_draft::allocate_formal_content_rev(&state.session_db, ds_id, now)
        .await
        .map_err(draft_err)?;
    let rev = project_config_draft::revision_row_from_config_row(&row, &saved, note);
    archive_project_config_revision(&state, rev).await?;
    let active = project_config_draft::close_draft_to_stable(
        &state.session_db,
        ds_id,
        &prev_stable,
        &git_sync_json,
    )
    .await
    .map_err(draft_err)?;
    Ok(Json(CommitProjectConfigDraftResponse {
        saved_content_rev: saved,
        activated: false,
        stable_content_rev: prev_stable,
        materialized: false,
        active_config: project_config_row_to_response(&state, active).await,
    }))
}

async fn get_gateway_global_settings_handler(
    State(state): State<AppState>,
) -> Result<Json<gateway_global_settings::GatewayGlobalSettingsResponse>, ApiError> {
    let body = gateway_global_settings::load_response(&state.session_db)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(body))
}

async fn upsert_gateway_git_pat_handler(
    State(state): State<AppState>,
    Json(req): Json<gateway_global_settings::PutGitPatInput>,
) -> Result<Json<gateway_global_settings::GitPatPublic>, ApiError> {
    let pat = gateway_global_settings::upsert_git_pat(&state.session_db, req)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    Ok(Json(pat))
}

async fn delete_gateway_git_pat_handler(
    State(state): State<AppState>,
    AxumPath(pat_id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = gateway_global_settings::delete_git_pat(&state.session_db, &pat_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "git PAT not found"))
    }
}

#[derive(Debug, Deserialize)]
struct PatchProjectConfigVersionNoteRequest {
    #[serde(default)]
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct PatchProjectConfigVersionNoteResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "contentRev")]
    content_rev: String,
    #[serde(rename = "note", skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    saved: bool,
}

async fn patch_project_config_version_note(
    State(state): State<AppState>,
    AxumPath((ds_id, content_rev)): AxumPath<(i64, String)>,
    Json(req): Json<PatchProjectConfigVersionNoteRequest>,
) -> Result<Json<PatchProjectConfigVersionNoteResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let content_rev = content_rev.trim();
    if content_rev.is_empty() || project_config_draft::is_draft_content_rev(content_rev) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cannot set note on draft revision",
        ));
    }
    project_config_draft::require_formal_revision(&state.session_db, ds_id, content_rev)
        .await
        .map_err(draft_err)?;
    let note = project_config_draft::normalize_revision_note(req.note);
    let saved = state
        .session_db
        .update_project_config_revision_note(ds_id, content_rev, note.as_deref())
        .await
        .map_err(|e| session_db_err(&e))?;
    if !saved {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no revision {content_rev} for ds {ds_id}"),
        ));
    }
    Ok(Json(PatchProjectConfigVersionNoteResponse {
        ds_id,
        content_rev: content_rev.to_string(),
        note,
        saved: true,
    }))
}

#[derive(Debug, Serialize)]
struct DeleteProjectConfigVersionResponse {
    #[serde(rename = "dsId")]
    ds_id: i64,
    #[serde(rename = "contentRev")]
    content_rev: String,
    deleted: bool,
}

async fn delete_project_config_version(
    State(state): State<AppState>,
    AxumPath((ds_id, content_rev)): AxumPath<(i64, String)>,
) -> Result<Json<DeleteProjectConfigVersionResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let content_rev = content_rev.trim();
    if content_rev.is_empty() || project_config_draft::is_draft_content_rev(content_rev) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cannot delete draft revision",
        ));
    }
    let row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    let Some(row) = row else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for ds {ds_id}"),
        ));
    };
    let effective = project_config_draft::effective_formal_rev(&row).map_err(draft_err)?;
    if content_rev == effective {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "cannot delete the effective contentRev; activate another version first",
        ));
    }
    let deleted = state
        .session_db
        .delete_project_config_revision(ds_id, content_rev)
        .await
        .map_err(|e| session_db_err(&e))?;
    if !deleted {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no revision {content_rev} for ds {ds_id}"),
        ));
    }
    Ok(Json(DeleteProjectConfigVersionResponse {
        ds_id,
        content_rev: content_rev.to_string(),
        deleted: true,
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
    if let Some(row) = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        if let Some(text) = row.claude_md.filter(|s| !s.trim().is_empty()) {
            return Ok(Json(ProjectClaudeResponse {
                ds_id,
                work_dir: work_dir.display().to_string(),
                path: home_claude_md_path.display().to_string(),
                exists: true,
                content: text,
            }));
        }
    }
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
    let lock = get_ds_lock(&state, ds_id).await;
    let _guard = lock.lock().await;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let Some(_) = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for ds {ds_id}; create project first"),
        ));
    };
    project_config_draft::ensure_draft(&state.session_db, ds_id)
        .await
        .map_err(draft_err)?;
    let mut row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists");
    row.claude_md = Some(req.content.clone());
    row.draft_open = true;
    row.content_rev = project_config_draft::DRAFT_CONTENT_REV.to_string();
    row.updated_at_ms = now_ms();
    let saved = req.content.clone();
    state
        .session_db
        .upsert_project_config(project_config_draft::upsert_from_row(
            &row,
            project_config_draft::DRAFT_CONTENT_REV,
            row.updated_at_ms,
            row.claude_md.as_deref(),
            row.stable_content_rev.as_deref(),
        ))
        .await
        .map_err(|e| session_db_err(&e))?;
    let now = row.updated_at_ms;
    project_entity_revision::append_claude(&state.session_db, ds_id, &saved, now)
        .await
        .map_err(entity_revision_err)?;
    let claude_md_path = work_dir.join("home/CLAUDE.md");
    Ok(Json(ProjectClaudeResponse {
        ds_id,
        work_dir: work_dir.display().to_string(),
        path: claude_md_path.display().to_string(),
        exists: true,
        content: saved,
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
    let lock = get_ds_lock(&state, ds_id).await;
    let _guard = lock.lock().await;
    ensure_workspace_initialized(&state.cfg.claw_bin, &work_dir).await?;
    let Some(_) = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("no project_config for ds {ds_id}; create project first"),
        ));
    };
    project_config_draft::ensure_draft(&state.session_db, ds_id)
        .await
        .map_err(draft_err)?;
    let mut row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists");
    let existed = row
        .skills_json
        .as_array()
        .is_some_and(|a| a.iter().any(|item| {
            item.get("skillName").and_then(Value::as_str) == Some(skill_name.as_str())
        }));
    merge_skill_into_skills_json(&mut row.skills_json, &skill_name, &req.skill_content);
    row.draft_open = true;
    row.content_rev = project_config_draft::DRAFT_CONTENT_REV.to_string();
    row.updated_at_ms = now_ms();
    let skill_body = row
        .skills_json
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|item| item.get("skillName").and_then(Value::as_str) == Some(skill_name.as_str()))
                .cloned()
        })
        .unwrap_or_else(|| {
            json!({
                "skillName": skill_name,
                "skillContent": req.skill_content,
            })
        });
    state
        .session_db
        .upsert_project_config(project_config_draft::upsert_from_row(
            &row,
            project_config_draft::DRAFT_CONTENT_REV,
            row.updated_at_ms,
            row.claude_md.as_deref(),
            row.stable_content_rev.as_deref(),
        ))
        .await
        .map_err(|e| session_db_err(&e))?;
    project_entity_revision::append_skill(
        &state.session_db,
        ds_id,
        &skill_name,
        skill_body,
        row.updated_at_ms,
    )
    .await
    .map_err(entity_revision_err)?;
    Ok(Json(ProjectSkillResponse {
        ds_id,
        skill_name,
        skill_path: skill_path.display().to_string(),
        created: !existed,
        updated: existed,
        bytes_written: req.skill_content.len(),
        work_dir: work_dir.display().to_string(),
    }))
}

/// Admin preview: always materialize latest `project_config` before assembling prompt.
async fn get_effective_prompt(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<EffectivePromptResponse>, ApiError> {
    build_effective_prompt_response(&state, ds_id, true)
        .await
        .map(Json)
}

async fn post_effective_prompt(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<EffectivePromptResponse>, ApiError> {
    build_effective_prompt_response(&state, ds_id, true)
        .await
        .map(Json)
}

async fn build_effective_prompt_response(
    state: &AppState,
    ds_id: i64,
    force_apply: bool,
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

    let row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if let Some(text) = row
        .as_ref()
        .and_then(|r| r.claude_md.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let message = text.to_string();
        if force_apply {
            let scaffold = gateway_global_settings::load_system_prompt_default(&state.session_db)
                .await
                .map_err(|e| session_db_err(&e))?;
            if let Some(ref r) = row {
                project_config_apply::apply_if_needed(&work_dir, r, true, &scaffold)
                    .await
                    .map_err(|e| map_project_config_apply_err(&e))?;
            }
        }
        return Ok(EffectivePromptResponse {
            ds_id,
            work_dir: work_dir.display().to_string(),
            sections: vec![message.clone()],
            message,
            prompt_source: "user".to_string(),
        });
    }

    apply_project_config_for_ds(state, ds_id, force_apply).await?;
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
        prompt_source: "system".to_string(),
    })
}

fn is_safe_skill_dir_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

async fn load_skills_from_ds_workdir(work_dir: &Path) -> std::io::Result<Vec<DsSkillEntry>> {
    let skills_root = work_dir.join("home").join("skills");
    let mut out = Vec::new();
    if !fs::metadata(&skills_root).await.is_ok_and(|m| m.is_dir()) {
        return Ok(out);
    }
    let mut rd = fs::read_dir(&skills_root).await?;
    while let Some(entry) = rd.next_entry().await? {
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let skill_name = entry.file_name().to_string_lossy().to_string();
        if !is_safe_skill_dir_name(&skill_name) {
            continue;
        }
        let path = entry.path().join("SKILL.md");
        if !fs::metadata(&path).await.is_ok_and(|m| m.is_file()) {
            continue;
        }
        let skill_content = fs::read_to_string(&path).await?;
        out.push(DsSkillEntry {
            skill_name,
            skill_content,
        });
    }
    out.sort_by(|a, b| a.skill_name.cmp(&b.skill_name));
    Ok(out)
}

async fn list_ds_skills(
    State(state): State<AppState>,
    AxumPath(ds_id): AxumPath<i64>,
) -> Result<Json<DsSkillsListResponse>, ApiError> {
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
    if let Some(row) = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        if let Some(arr) = row.skills_json.as_array() {
            if !arr.is_empty() {
                let mut skills = Vec::new();
                for item in arr {
                    let Some(obj) = item.as_object() else {
                        continue;
                    };
                    let Some(name) = obj.get("skillName").and_then(Value::as_str) else {
                        continue;
                    };
                    let content = obj
                        .get("skillContent")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    skills.push(DsSkillEntry {
                        skill_name: name.to_string(),
                        skill_content: content,
                    });
                }
                skills.sort_by(|a, b| a.skill_name.cmp(&b.skill_name));
                return Ok(Json(DsSkillsListResponse { ds_id, skills }));
            }
        }
    }
    let skills = load_skills_from_ds_workdir(&work_dir).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("list skills failed: {e}"),
        )
    })?;
    Ok(Json(DsSkillsListResponse { ds_id, skills }))
}

async fn get_ds_skill(
    State(state): State<AppState>,
    AxumPath((ds_id, skill_name)): AxumPath<(i64, String)>,
) -> Result<Json<DsSkillGetResponse>, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    if !is_safe_skill_dir_name(&skill_name) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid skill_name"));
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
    if let Some(row) = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        if let Some(arr) = row.skills_json.as_array() {
            for item in arr {
                if item.get("skillName").and_then(Value::as_str) == Some(skill_name.as_str()) {
                    let content = item
                        .get("skillContent")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    return Ok(Json(DsSkillGetResponse {
                        ds_id,
                        skill_name,
                        skill_content: content,
                    }));
                }
            }
        }
    }
    let path = work_dir
        .join("home")
        .join("skills")
        .join(&skill_name)
        .join("SKILL.md");
    let skill_content = match fs::read_to_string(&path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                format!("skill not found: {skill_name}"),
            ));
        }
        Err(e) => {
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("read skill failed: {e}"),
            ));
        }
    };
    Ok(Json(DsSkillGetResponse {
        ds_id,
        skill_name,
        skill_content,
    }))
}

fn progress_poll_interval_ms() -> u64 {
    std::env::var("CLAW_TASK_PROGRESS_POLL_MS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .filter(|&n| n >= 100)
        .unwrap_or(400)
}

fn gateway_queue_snapshot(tasks: &HashMap<String, TaskInner>) -> task_status::GatewayQueueSnapshot {
    let rows: HashMap<String, TaskStatusRow> = tasks
        .iter()
        .map(|(id, inner)| {
            (
                id.clone(),
                TaskStatusRow {
                    status: inner.record.status.clone(),
                },
            )
        })
        .collect();
    count_gateway_tasks(&rows)
}

async fn resolve_session_home_path(
    state: &AppState,
    ds_id: i64,
    session_id: &str,
) -> Option<PathBuf> {
    let rel = state
        .session_db
        .get_session_home_rel(session_id, ds_id)
        .await
        .ok()??;
    session_merge::validate_session_home_rel(&rel).ok()?;
    Some(join_session_home(&state.cfg.work_root, &rel))
}

async fn refresh_task_progress(state: &AppState, task_id: &str) {
    let snapshot = {
        let (status, ds_id, session_id) = {
            let tasks = state.tasks.lock().await;
            let Some(inner) = tasks.get(task_id) else {
                return;
            };
            (
                inner.record.status.clone(),
                inner.ds_id,
                inner.record.session_id.clone(),
            )
        };
        let session_home = resolve_session_home_path(state, ds_id, &session_id).await;
        let queue = {
            let tasks = state.tasks.lock().await;
            gateway_queue_snapshot(&tasks)
        };
        let trace_paths = session_home
            .as_ref()
            .map(|home| discover_trace_paths(home, &state.cfg.work_root, &session_id))
            .unwrap_or_default();
        let tool = trace_tail_suggests_tool_call(&trace_paths);
        let desc = resolve_current_task_desc(&status, session_home.as_deref(), &queue, tool);
        let updated_ms = session_home
            .as_ref()
            .and_then(|home| read_task_progress(home))
            .map(|p| p.updated_at_ms);
        (desc, updated_ms)
    };
    let mut tasks = state.tasks.lock().await;
    if let Some(inner) = tasks.get_mut(task_id) {
        inner.record.current_task_desc = snapshot.0;
        inner.record.progress_updated_at_ms = snapshot.1;
    }
}

fn spawn_task_progress_poller(state: AppState, task_id: String) {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_millis(progress_poll_interval_ms()));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let active = {
                let tasks = state.tasks.lock().await;
                tasks.get(&task_id).is_some_and(|inner| {
                    matches!(inner.record.status.as_str(), "queued" | "running")
                })
            };
            if !active {
                break;
            }
            refresh_task_progress(&state, &task_id).await;
        }
    });
}

#[derive(Debug, Deserialize)]
struct SessionExecutionQuery {
    #[serde(rename = "ds_id")]
    ds_id: i64,
    #[serde(default)]
    include_trace: bool,
}

async fn get_session_execution(
    State(state): State<AppState>,
    AxumPath(session_id): AxumPath<String>,
    Query(query): Query<SessionExecutionQuery>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<SessionExecutionResponse>, ApiError> {
    if query.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "ds_id must be >= 1"));
    }
    let session_home_rel = state
        .session_db
        .get_session_home_rel(&session_id, query.ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("session not found: {session_id} ds_id={}", query.ds_id),
            )
        })?;
    session_merge::validate_session_home_rel(&session_home_rel).map_err(session_routing_error)?;
    let session_home = join_session_home(&state.cfg.work_root, &session_home_rel);

    refresh_task_progress(&state, &session_id).await;

    let (record_opt, queue) = {
        let tasks = state.tasks.lock().await;
        let queue = gateway_queue_snapshot(&tasks);
        let record = tasks.get(&session_id).map(|inner| inner.record.clone());
        (record, queue)
    };
    let task_snapshot = if let Some(record) = record_opt {
        let has_report = task_has_report(&state.session_db, &record).await;
        Some(SessionExecutionTask {
            task_id: record.task_id.clone(),
            status: record.status.clone(),
            has_report,
            created_at_ms: record.created_at_ms,
            started_at_ms: record.started_at_ms,
            finished_at_ms: record.finished_at_ms,
            current_task_desc: record.current_task_desc.clone(),
        })
    } else {
        None
    };

    let task = task_snapshot.unwrap_or_else(|| SessionExecutionTask {
        task_id: session_id.clone(),
        status: "unknown".to_string(),
        has_report: false,
        created_at_ms: 0,
        started_at_ms: None,
        finished_at_ms: None,
        current_task_desc: None,
    });

    let progress = read_task_progress(&session_home);
    let progress_history = read_progress_events(&session_home, 50)
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let trace_paths = discover_trace_paths(&session_home, &state.cfg.work_root, &session_id);
    let trace_tail = if query.include_trace {
        read_trace_tail(&trace_paths, 50, true)
    } else {
        read_trace_tail(&trace_paths, 20, false)
    };

    info!(
        request_id = %http_request_id.0,
        session_id = %session_id,
        ds_id = query.ds_id,
        endpoint = "/v1/sessions/{session_id}/execution",
        "gateway_session_execution"
    );

    Ok(Json(SessionExecutionResponse {
        session_id,
        ds_id: query.ds_id,
        session_home_rel,
        task,
        progress,
        progress_history,
        queue,
        trace_tail,
    }))
}

fn solve_async_response_headers(
    effective: &str,
) -> Result<AppendHeaders<[(header::HeaderName, HeaderValue); 2]>, ApiError> {
    let claw = HeaderValue::from_str(effective).map_err(|_| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid characters in session id for response header",
        )
    })?;
    let xrid = header::HeaderName::from_static("x-request-id");
    let csid = header::HeaderName::from_static("claw-session-id");
    Ok(AppendHeaders([(xrid, claw.clone()), (csid, claw)]))
}

async fn enqueue_solve_async(
    state: AppState,
    http_request_id: HttpRequestId,
    id_kind: session_merge::HttpRequestIdKind,
    req: SolveRequest,
    endpoint: &'static str,
) -> Result<SolveAsyncResponse, ApiError> {
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
    let ds_id = req.ds_id;
    let new_turn_id = turn_id::mint_turn_id();
    register_solve_turn(
        &state.session_db,
        &new_turn_id,
        &effective,
        ds_id,
        &req.user_prompt,
    )
    .await?;
    if let Some(rel) = state
        .session_db
        .get_session_home_rel(&effective, ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        let home = join_session_home(&state.cfg.work_root, &rel);
        if let Err(e) = reset_task_progress(&home, &effective) {
            warn!(error = %e, "reset task progress before async solve failed");
        }
        let _ = truncate_progress_history(&home);
    }
    info!(
        request_id = %effective,
        task_id = %task_id,
        ds_id = req.ds_id,
        endpoint,
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
        let queue = gateway_queue_snapshot(&tasks);
        let initial_desc = resolve_current_task_desc("queued", None, &queue, false);
        tasks.insert(
            task_id.clone(),
            TaskInner {
                record: TaskRecord {
                    task_id: task_id.clone(),
                    session_id: effective.clone(),
                    request_id: effective.clone(),
                    ds_id,
                    status: "queued".to_string(),
                    created_at_ms: now_ms(),
                    started_at_ms: None,
                    finished_at_ms: None,
                    current_task_desc: initial_desc,
                    progress_updated_at_ms: None,
                    result: None,
                    error: None,
                    turn_id: new_turn_id.clone(),
                    progress_history: Vec::new(),
                    has_report: false,
                },
                cancel: None,
                ds_id,
            },
        );
    }
    spawn_task_progress_poller(state.clone(), task_id.clone());
    let state_clone = state.clone();
    let task_id_for_worker = task_id.clone();
    let rid = effective.clone();
    let turn_id_for_worker = new_turn_id.clone();
    let join = tokio::spawn(async move {
        {
            let mut tasks = state_clone.tasks.lock().await;
            if let Some(inner) = tasks.get_mut(&task_id_for_worker) {
                if inner.record.status == "cancelled" {
                    inner.cancel = None;
                    finalize_solve_turn_cancelled(&state_clone.session_db, &turn_id_for_worker)
                        .await;
                    return;
                }
                inner.record.status = "running".to_string();
                inner.record.started_at_ms = Some(now_ms());
            }
        }
        set_solve_turn_status(
            &state_clone.session_db,
            &turn_id_for_worker,
            "running",
            false,
        )
        .await;
        info!(
            request_id = %rid,
            task_id = %task_id_for_worker,
            turn_id = %turn_id_for_worker,
            phase = "running",
            "gateway_solve_async"
        );
        let result = run_solve_request(
            state_clone.clone(),
            req,
            RunSolveContext {
                request_id: rid.clone(),
                task_id: Some(task_id_for_worker.clone()),
                turn_id: turn_id_for_worker.clone(),
                skip_session_db: false,
            },
        )
        .await;
        let refresh_progress = {
            let mut tasks = state_clone.tasks.lock().await;
            let Some(inner) = tasks.get_mut(&task_id_for_worker) else {
                return;
            };
            inner.cancel = None;
            if inner.record.status == "cancelled" {
                finalize_solve_turn_cancelled(&state_clone.session_db, &turn_id_for_worker).await;
                return;
            }
            inner.record.finished_at_ms = Some(now_ms());
            match result {
                Ok(ref v) => {
                    let duration_ms = v.duration_ms;
                    inner.record.status = "succeeded".to_string();
                    inner.record.result = Some(v.clone());
                    finalize_solve_turn_success(
                        Arc::clone(&state_clone.session_db),
                        &turn_id_for_worker,
                        v,
                    )
                        .await;
                    info!(
                        request_id = %rid,
                        task_id = %task_id_for_worker,
                        phase = "succeeded",
                        duration_ms,
                        "gateway_solve_async"
                    );
                }
                Err(ref e) => {
                    inner.record.status = "failed".to_string();
                    inner.record.error =
                        Some(json!({"status_code": e.status.as_u16(), "detail": e.message}));
                    finalize_solve_turn_failed(&state_clone.session_db, &turn_id_for_worker, e)
                        .await;
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
            true
        };
        if refresh_progress {
            refresh_task_progress(&state_clone, &task_id_for_worker).await;
        }
    });
    let cancel = join.abort_handle();
    {
        let mut tasks = state.tasks.lock().await;
        if let Some(inner) = tasks.get_mut(&task_id) {
            inner.cancel = Some(cancel);
        }
    }
    refresh_task_progress(&state, &task_id).await;
    Ok(SolveAsyncResponse {
        task_id: task_id.clone(),
        session_id: effective.clone(),
        request_id: effective.clone(),
        turn_id: new_turn_id,
        status: "queued".to_string(),
        poll_url: format!("/v1/tasks/{task_id}"),
    })
}

async fn solve_start(
    State(state): State<AppState>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Extension(id_kind): Extension<session_merge::HttpRequestIdKind>,
    Json(req): Json<StartRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let body_sid = session_merge::trim_session_id(req.session_id.as_deref());
    let effective =
        session_merge::merge_effective_session_id(body_sid, &http_request_id.0, id_kind)
            .map_err(session_routing_error)?;
    if body_sid.is_some() {
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
    prepare_gateway_session(
        &state,
        req.ds_id,
        req.session_id.as_deref(),
        req.extra_session.as_ref(),
        &effective,
        false,
    )
    .await?;
    info!(
        request_id = %effective,
        ds_id = req.ds_id,
        endpoint = "/v1/start",
        phase = "session_ready",
        "gateway_start: session registered in SQLite before response"
    );
    let headers = solve_async_response_headers(&effective)?;
    Ok((
        headers,
        Json(SolveStartResponse {
            session_id: effective.clone(),
            request_id: effective,
        }),
    ))
}

async fn solve_async(
    State(state): State<AppState>,
    Extension(http_request_id): Extension<HttpRequestId>,
    Extension(id_kind): Extension<session_merge::HttpRequestIdKind>,
    Json(req): Json<SolveRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let out = enqueue_solve_async(state, http_request_id, id_kind, req, "/v1/solve_async").await?;
    let headers = solve_async_response_headers(&out.session_id)?;
    Ok((headers, Json(out)))
}

/// In-memory async task row, or after gateway restart the latest `gateway_turns` row for this
/// `session_id` (`task_id`). Author: kejiqing
async fn try_load_task_record(
    state: &AppState,
    task_id: &str,
) -> Result<Option<(TaskRecord, i64)>, ApiError> {
    {
        let tasks = state.tasks.lock().await;
        if let Some(inner) = tasks.get(task_id) {
            return Ok(Some((inner.record.clone(), inner.ds_id)));
        }
    }
    let Some(row) = state
        .session_db
        .fetch_latest_turn_for_session(task_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Ok(None);
    };
    Ok(Some(
        task_record_from_latest_turn_row(state, task_id, row).await?,
    ))
}

async fn task_record_from_latest_turn_row(
    state: &AppState,
    task_id: &str,
    row: session_db::LatestTurnRow,
) -> Result<(TaskRecord, i64), ApiError> {
    let session_home_rel = state
        .session_db
        .get_session_home_rel(task_id, row.ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .unwrap_or_default();
    let work_dir = join_session_home(&state.cfg.work_root, &session_home_rel)
        .to_string_lossy()
        .to_string();
    let duration_ms = row
        .finished_at_ms
        .unwrap_or(row.created_at_ms)
        .saturating_sub(row.created_at_ms);
    let output_text = row
        .report_message
        .clone()
        .or_else(|| {
            row.output_json.as_ref().and_then(|j| {
                j.get("message")
                    .and_then(Value::as_str)
                    .map(std::string::ToString::to_string)
            })
        })
        .unwrap_or_default();
    let result = if row.status == "succeeded" {
        Some(SolveResponse {
            session_id: task_id.to_string(),
            request_id: task_id.to_string(),
            session_home_rel: session_home_rel.clone(),
            ds_id: row.ds_id,
            work_dir,
            duration_ms,
            claw_exit_code: row.claw_exit_code.unwrap_or(0),
            output_text,
            output_json: row.output_json.clone(),
            turn_id: row.turn_id.clone(),
        })
    } else {
        None
    };
    let error = if row.status == "failed" {
        row.output_json
            .clone()
            .or_else(|| Some(json!({"detail": "solve turn failed"})))
    } else if row.status == "cancelled" {
        Some(json!({"detail":"cancelled by client","outcome":"cancelled"}))
    } else {
        None
    };
    let session_home = resolve_session_home_path(state, row.ds_id, task_id).await;
    let queue = {
        let tasks = state.tasks.lock().await;
        gateway_queue_snapshot(&tasks)
    };
    let trace_paths = session_home
        .as_ref()
        .map(|home| discover_trace_paths(home, &state.cfg.work_root, task_id))
        .unwrap_or_default();
    let tool = trace_tail_suggests_tool_call(&trace_paths);
    let current_task_desc =
        resolve_current_task_desc(&row.status, session_home.as_deref(), &queue, tool);
    let progress_updated_at_ms = session_home
        .as_ref()
        .and_then(|home| read_task_progress(home))
        .map(|p| p.updated_at_ms);
    let mut record = TaskRecord {
        task_id: task_id.to_string(),
        session_id: task_id.to_string(),
        request_id: task_id.to_string(),
        ds_id: row.ds_id,
        status: row.status.clone(),
        created_at_ms: row.created_at_ms,
        started_at_ms: Some(row.created_at_ms),
        finished_at_ms: row.finished_at_ms,
        current_task_desc,
        progress_updated_at_ms,
        result,
        error,
        turn_id: row.turn_id.clone(),
        progress_history: Vec::new(),
        has_report: false,
    };
    if let Some(ref home) = session_home {
        record.progress_history = read_progress_events(home, 50)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        let _ = home;
    }
    record.has_report = task_has_report(&state.session_db, &record).await;
    let ds_id = record.ds_id;
    Ok((record, ds_id))
}

async fn get_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<TaskRecord>, ApiError> {
    refresh_task_progress(&state, &task_id).await;
    let (mut task, ds_id) = try_load_task_record(&state, &task_id)
        .await?
        .ok_or_else(|| {
            ApiError::new(StatusCode::NOT_FOUND, format!("task not found: {task_id}"))
        })?;
    if let Some(home) = resolve_session_home_path(&state, ds_id, &task.session_id).await {
        task.progress_history = read_progress_events(&home, 50)
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }
    task.has_report = task_has_report(&state.session_db, &task).await;
    info!(
        request_id = %http_request_id.0,
        task_id = %task_id,
        task_request_id = %task.request_id,
        task_status = %task.status,
        has_report = task.has_report,
        progress_events = task.progress_history.len(),
        endpoint = "/v1/tasks/{task_id}",
        phase = "poll",
        "gateway_task"
    );
    Ok(Json(task))
}

async fn task_has_report(db: &session_db::GatewaySessionDb, task: &TaskRecord) -> bool {
    if task.status == "succeeded" {
        return true;
    }
    db.turn_has_live_chunks(&task.turn_id)
        .await
        .unwrap_or(false)
}

fn task_status_is_terminal_for_cancel(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

/// Terminal-state cancel is idempotent: HTTP 200, `error` explains no state change. kejiqing
fn task_cancel_idempotent_response(record: TaskRecord) -> TaskRecord {
    let status_at_cancel = record.status.clone();
    let previous_error = record.error.clone();
    let detail = match status_at_cancel.as_str() {
        "cancelled" => "task already cancelled; duplicate cancel ignored".to_string(),
        "succeeded" => "task already succeeded; cancel had no effect".to_string(),
        "failed" => "task already failed; cancel had no effect".to_string(),
        other => format!("task already in terminal state ({other}); cancel had no effect"),
    };
    let mut out = record;
    out.error = Some(json!({
        "detail": detail,
        "outcome": "idempotent",
        "cancelApplied": false,
        "statusAtCancel": status_at_cancel,
        "previousError": previous_error,
    }));
    out
}

async fn cancel_task_cold_db(
    state: &AppState,
    task_id: &str,
    http_request_id: &HttpRequestId,
) -> Result<Json<TaskRecord>, ApiError> {
    let Some(row) = state
        .session_db
        .fetch_latest_turn_for_session(task_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("task not found: {task_id}"),
        ));
    };
    if task_status_is_terminal_for_cancel(&row.status) {
        let (record, _) = task_record_from_latest_turn_row(state, task_id, row).await?;
        let task_status = record.status.clone();
        let out = task_cancel_idempotent_response(record);
        info!(
            request_id = %http_request_id.0,
            task_id = %task_id,
            task_status = %task_status,
            endpoint = "/v1/tasks/{task_id}/cancel",
            phase = "cancel_idempotent_db",
            "gateway_task"
        );
        return Ok(Json(out));
    }
    finalize_solve_turn_cancelled(&state.session_db, &row.turn_id).await;
    let Some(row2) = state
        .session_db
        .fetch_latest_turn_for_session(task_id)
        .await
        .map_err(|e| session_db_err(&e))?
    else {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "task row missing after cancel",
        ));
    };
    let (record, _) = task_record_from_latest_turn_row(state, task_id, row2).await?;
    info!(
        request_id = %http_request_id.0,
        task_id = %task_id,
        endpoint = "/v1/tasks/{task_id}/cancel",
        phase = "cancel_cold_db",
        "gateway_task"
    );
    Ok(Json(record))
}

async fn cancel_task(
    State(state): State<AppState>,
    AxumPath(task_id): AxumPath<String>,
    Extension(http_request_id): Extension<HttpRequestId>,
) -> Result<Json<TaskRecord>, ApiError> {
    let cancel_handle = {
        let mut tasks = state.tasks.lock().await;
        let Some(inner) = tasks.get_mut(&task_id) else {
            return cancel_task_cold_db(&state, &task_id, &http_request_id).await;
        };
        if task_status_is_terminal_for_cancel(&inner.record.status) {
            let task_status = inner.record.status.clone();
            let record = task_cancel_idempotent_response(inner.record.clone());
            info!(
                request_id = %http_request_id.0,
                task_id = %task_id,
                task_status = %task_status,
                endpoint = "/v1/tasks/{task_id}/cancel",
                phase = "cancel_idempotent",
                "gateway_task"
            );
            return Ok(Json(record));
        }
        let h = inner.cancel.take();
        inner.record.status = "cancelled".to_string();
        inner.record.finished_at_ms = Some(now_ms());
        inner.record.result = None;
        inner.record.error = Some(json!({
            "detail": "cancelled by client",
            "outcome": "cancelled",
            "cancelApplied": true,
        }));
        h
    };
    // Stop the container worker before aborting the host task: `kill_on_drop` then tears down
    // the `docker exec` client, and in-flight stderr can still flush while the container exits.
    if let Some((pool, idx)) = state.docker_slots.lock().await.remove(&task_id) {
        let _ = pool.force_kill_slot(idx).await;
    }
    if let Some(h) = cancel_handle {
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
    finalize_solve_turn_cancelled(&state.session_db, &record.turn_id).await;
    Ok(Json(record))
}

/// `CLAW_GATEWAY_DEV_BIZ_REPORT_SEED=1` only; otherwise 404.
async fn dev_seed_biz_report_task(
    State(state): State<AppState>,
    Json(body): Json<DevBizReportSeedRequest>,
) -> Result<Json<Value>, ApiError> {
    if std::env::var("CLAW_GATEWAY_DEV_BIZ_REPORT_SEED")
        .ok()
        .as_deref()
        != Some("1")
    {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not found"));
    }
    if body.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let tid = body
        .task_id
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map_or_else(|| Uuid::new_v4().simple().to_string(), ToString::to_string);
    let work_dir = ds_work_dir(&state.cfg.work_root, body.ds_id);
    let now = now_ms();
    let output_text = if body.output_text.trim().is_empty() {
        "mock raw boss output for polish".to_string()
    } else {
        body.output_text.clone()
    };
    let seed_turn_id = turn_id::mint_turn_id();
    let result = SolveResponse {
        session_id: tid.clone(),
        request_id: tid.clone(),
        session_home_rel: format!("ds_{}/sessions/dev-seed", body.ds_id),
        ds_id: body.ds_id,
        work_dir: work_dir.to_string_lossy().to_string(),
        duration_ms: 0,
        claw_exit_code: 0,
        output_text,
        output_json: body.output_json.clone(),
        turn_id: seed_turn_id.clone(),
    };
    let record = TaskRecord {
        task_id: tid.clone(),
        session_id: tid.clone(),
        request_id: tid.clone(),
        ds_id: body.ds_id,
        status: "succeeded".to_string(),
        created_at_ms: now,
        started_at_ms: Some(now),
        finished_at_ms: Some(now),
        current_task_desc: Some("分析完成".to_string()),
        progress_updated_at_ms: Some(now),
        result: Some(result),
        error: None,
        turn_id: seed_turn_id.clone(),
        progress_history: Vec::new(),
        has_report: false,
    };
    {
        let mut tasks = state.tasks.lock().await;
        tasks.insert(
            tid.clone(),
            TaskInner {
                record,
                cancel: None,
                ds_id: body.ds_id,
            },
        );
    }
    let stream_url = format!(
        "/v1/biz_advice_report?sessionId={tid}&turnId={seed_turn_id}&dsId={}&stream=true",
        body.ds_id
    );
    Ok(Json(json!({
        "taskId": tid,
        "bizAdviceReportStreamUrl": stream_url,
    })))
}

async fn prepare_live_report_context(
    state: &AppState,
    session_id: &str,
    turn_id: &str,
    ds_id: i64,
) -> Result<LiveReportContext, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    if !turn_id::validate_turn_id(turn_id) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "turnId must match T_<32 hex>",
        ));
    }
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId must be non-empty",
        ));
    }
    if !state
        .session_db
        .session_exists(session_id, ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            format!("session not found: {session_id} ds_id={ds_id}"),
        ));
    }
    if !state
        .session_db
        .turn_belongs_to_session(turn_id, session_id, ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unknown turnId for session",
        ));
    }
    let session_home_rel = state
        .session_db
        .get_session_home_rel(session_id, ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!("session not found: {session_id} ds_id={ds_id}"),
            )
        })?;
    session_merge::validate_session_home_rel(&session_home_rel).map_err(session_routing_error)?;
    let session_home = join_session_home(&state.cfg.work_root, &session_home_rel);
    Ok(LiveReportContext {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        ds_id,
        session_home,
    })
}

async fn internal_assistant_stream(
    State(state): State<AppState>,
    AxumPath(turn_id): AxumPath<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Body,
) -> Result<Response, ApiError> {
    turn_live::post_assistant_stream(state, turn_id, headers, body).await
}

async fn get_biz_advice_report(
    State(state): State<AppState>,
    Query(query): Query<BizAdviceReportQuery>,
) -> Result<Response, ApiError> {
    let state = Arc::new(state);
    let ctx =
        prepare_live_report_context(&state, &query.session_id, &query.turn_id, query.ds_id).await?;
    let use_live_pg = should_use_live_pg_report(state.as_ref(), &ctx).await?;
    if !use_live_pg {
        return respond_biz_advice_polish_for_context(state, ctx, query.stream).await;
    }
    if query.stream {
        let rx = spawn_live_report_sse_worker(Arc::clone(&state), ctx.clone());
        let no_buffer = header::HeaderName::from_static("x-accel-buffering");
        let no_buffer_val = HeaderValue::from_static("no");
        return Ok((
            AppendHeaders([(no_buffer, no_buffer_val)]),
            Sse::new(biz_report_sse_event_stream(&ctx.session_id, rx))
                .keep_alive(KeepAlive::default()),
        )
            .into_response());
    }
    let (report_text, report_json) =
        biz_advice_report_live::live_report_json_response(&state, ctx.clone()).await?;
    let (report_text, report_json) = sanitize_biz_report_parts(&report_text, Some(report_json));
    let status = biz_advice_report_live::turn_status(
        &state.session_db,
        &ctx.turn_id,
        &ctx.session_id,
        ctx.ds_id,
    )
    .await?
    .unwrap_or_else(|| "succeeded".to_string());
    Ok(Json(BizAdviceReportResponse {
        task_id: ctx.session_id.clone(),
        source_request_id: ctx.session_id,
        source_ds_id: ctx.ds_id,
        source_status: status,
        report_text,
        report_json,
    })
    .into_response())
}

/// No spill file: polish solve `outputJson.message` (legacy `_bak` path) for JSON or SSE.
async fn respond_biz_advice_polish_for_context(
    state: Arc<AppState>,
    ctx: LiveReportContext,
    stream: bool,
) -> Result<Response, ApiError> {
    let status = biz_advice_report_live::turn_status(
        &state.session_db,
        &ctx.turn_id,
        &ctx.session_id,
        ctx.ds_id,
    )
    .await?;
    let Some(status) = status else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown turnId for session",
        ));
    };
    if status != "succeeded" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("turn not finished yet (status: {status})"),
        ));
    }
    let report_body =
        biz_advice_report_live::resolve_formal_report_text(state.as_ref(), &ctx).await?;
    let skill_work_dir = ds_work_dir(&state.cfg.work_root, BOSS_REPORT_SKILL_DS_ID);
    ensure_workspace_initialized(&state.cfg.claw_bin, &skill_work_dir).await?;
    let instructions = load_boss_report_writer_instructions(&skill_work_dir).await;
    let prompt = build_biz_advice_polish_prompt(&instructions, &report_body);
    let request_id = Uuid::new_v4().simple().to_string();
    let timeout_seconds = state.cfg.default_timeout_seconds;
    let polish_ds = state.cfg.report_polish_deepseek.clone();
    let meta = BizAdviceReportPayload {
        task_id: ctx.session_id.clone(),
        source_request_id: ctx.session_id.clone(),
        source_ds_id: ctx.ds_id,
        source_status: status,
        report_text: None,
        report_json: None,
    };
    if stream {
        return Ok(biz_report_llm_stream_response(
            &ctx.session_id,
            meta,
            prompt,
            request_id,
            timeout_seconds,
            polish_ds,
        ));
    }
    let (report_text, report_json) = tokio::task::spawn_blocking(move || {
        run_gateway_biz_polish_llm(
            &prompt,
            None,
            timeout_seconds,
            &request_id,
            None::<fn(&str)>,
            polish_ds.as_ref(),
        )
    })
    .await
    .map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("polish task join failed: {e}"),
        )
    })?
    .map_err(map_gateway_solve_turn_err)?;
    let (report_text, report_json) = sanitize_biz_report_parts(&report_text, report_json);
    Ok(Json(BizAdviceReportResponse {
        task_id: ctx.session_id.clone(),
        source_request_id: ctx.session_id,
        source_ds_id: ctx.ds_id,
        source_status: meta.source_status,
        report_text,
        report_json,
    })
    .into_response())
}

async fn get_biz_advice_report_bak(
    State(state): State<AppState>,
    Query(query): Query<BizAdviceReportBakQuery>,
) -> Result<Response, ApiError> {
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
    if source_result.claw_exit_code != 0 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "task {} did not complete successfully (clawExitCode: {})",
                query.task_id, source_result.claw_exit_code
            ),
        ));
    }
    let report_body = report_body_from_solve_output(
        &source_result.output_text,
        source_result.output_json.as_ref(),
    )
    .map_err(|detail| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("task {} has empty report message: {detail}", query.task_id),
        )
    })?;
    let ds_id = source_result.ds_id;
    let skill_work_dir = ds_work_dir(&state.cfg.work_root, BOSS_REPORT_SKILL_DS_ID);
    ensure_workspace_initialized(&state.cfg.claw_bin, &skill_work_dir).await?;
    let instructions = load_boss_report_writer_instructions(&skill_work_dir).await;
    let prompt = build_biz_advice_polish_prompt(&instructions, &report_body);
    let request_id = Uuid::new_v4().simple().to_string();
    let timeout_seconds = state.cfg.default_timeout_seconds;
    if query.stream {
        let meta = BizAdviceReportPayload {
            task_id: query.task_id.clone(),
            source_request_id: task.request_id.clone(),
            source_ds_id: ds_id,
            source_status: source_status.clone(),
            report_text: None,
            report_json: None,
        };
        let task_id = meta.task_id.clone();
        let polish_ds = state.cfg.report_polish_deepseek.clone();
        return Ok(biz_report_llm_stream_response(
            &task_id,
            meta,
            prompt,
            request_id,
            timeout_seconds,
            polish_ds,
        ));
    }
    let polish_ds = state.cfg.report_polish_deepseek.clone();
    let (report_text, report_json) = tokio::task::spawn_blocking(move || {
        run_gateway_biz_polish_llm(
            &prompt,
            None,
            timeout_seconds,
            &request_id,
            None::<fn(&str)>,
            polish_ds.as_ref(),
        )
    })
    .await
    .map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("polish task join failed: {e}"),
        )
    })?
    .map_err(map_gateway_solve_turn_err)?;
    let (report_text, report_json) = sanitize_biz_report_parts(&report_text, report_json);
    Ok(Json(BizAdviceReportResponse {
        task_id: query.task_id,
        source_request_id: task.request_id,
        source_ds_id: ds_id,
        source_status,
        report_text,
        report_json,
    })
    .into_response())
}

/// `stream=true`: direct LLM polish; each model `TextDelta` is forwarded as `biz.report.delta`.
fn biz_report_llm_stream_response(
    task_id: &str,
    meta_done: BizAdviceReportPayload,
    prompt: String,
    request_id: String,
    timeout_seconds: u64,
    report_polish_deepseek: Option<ReportPolishDeepseek>,
) -> Response {
    let (tx, rx) = mpsc::unbounded_channel::<BizReportStreamMsg>();
    tokio::spawn(async move {
        let mut export_sanitizer = ReportExportSanitizer::new(true);
        let mut send_delta = |delta: &str| {
            let clean = export_sanitizer.push_chunk(delta);
            if !clean.is_empty() {
                let _ = tx.send(BizReportStreamMsg::Delta(clean));
            }
        };
        match run_gateway_biz_polish_llm_async(
            &prompt,
            None,
            timeout_seconds,
            &request_id,
            Some(&mut send_delta),
            report_polish_deepseek.as_ref(),
        )
        .await
        {
            Ok((output_text, output_json)) => {
                let mut done = BizAdviceReportPayload {
                    task_id: meta_done.task_id,
                    source_request_id: meta_done.source_request_id,
                    source_ds_id: meta_done.source_ds_id,
                    source_status: meta_done.source_status,
                    report_text: Some(sanitize_external_report_text(&output_text)),
                    report_json: output_json,
                };
                sanitize_report_payload(&mut done);
                let _ = tx.send(BizReportStreamMsg::Done(done));
            }
            Err(e) => {
                let _ = tx.send(BizReportStreamMsg::Error(e.message));
            }
        }
    });
    let no_buffer = header::HeaderName::from_static("x-accel-buffering");
    let no_buffer_val = HeaderValue::from_static("no");
    (
        AppendHeaders([(no_buffer, no_buffer_val)]),
        Sse::new(biz_report_sse_event_stream(task_id, rx)).keep_alive(KeepAlive::default()),
    )
        .into_response()
}

fn validate_extra_session(extra_session: Option<&Value>) -> Result<(), ApiError> {
    if let Some(extra_session) = extra_session {
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
    Ok(())
}

fn validate_solve_request_fields(req: &SolveRequest) -> Result<(), ApiError> {
    if req.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    if req.user_prompt.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "userPrompt cannot be empty",
        ));
    }
    validate_extra_session(req.extra_session.as_ref())
}

/// Ensures `(sessionId, dsId)` exists in `SQLite` and session `.claw/settings.json` on disk. kejiqing
async fn prepare_gateway_session(
    state: &AppState,
    ds_id: i64,
    body_session_id: Option<&str>,
    extra_session: Option<&Value>,
    request_id: &str,
    skip_session_db: bool,
) -> Result<PreparedGatewaySession, ApiError> {
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    validate_extra_session(extra_session)?;
    validate_ds_exists(ds_id, &state.cfg.ds_registry_path).await?;

    let _session_lock_guard: Option<OwnedMutexGuard<()>> = if skip_session_db {
        None
    } else {
        Some(
            get_session_solve_lock(state, ds_id, request_id)
                .await
                .lock_owned()
                .await,
        )
    };

    let ds_base = state.cfg.work_root.join(format!("ds_{ds_id}"));
    let explicit_continuation = session_merge::trim_session_id(body_session_id).is_some();

    let (session_home, need_insert_row, purge_mcp_discovery, session_fs_label) = if skip_session_db
    {
        let session_fs_id = session_merge::sessions_directory_segment(request_id);
        let session_home = ds_base.join("sessions").join(&session_fs_id);
        (session_home, false, true, session_fs_id)
    } else {
        let row_opt = state
            .session_db
            .get_session_home_rel(request_id, ds_id)
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
            let session_fs_id = session_merge::sessions_directory_segment(request_id);
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
        let ds_lock = get_ds_lock(state, ds_id).await;
        let _guard = ds_lock.lock().await;
        ensure_ds_project_ready(state, ds_id).await?;
        fs::create_dir_all(&ds_base).await.map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create ds dir failed: {e}"),
            )
        })?;
        ensure_workspace_initialized(&state.cfg.claw_bin, &ds_base).await?;
        let settings = build_settings(state, ds_id).await;
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
            .insert_session(request_id, ds_id, &session_home_rel, now_ms())
            .await
            .map_err(|e| session_db_err(&e))?;
    } else if !skip_session_db {
        state
            .session_db
            .touch_updated(request_id, ds_id, now_ms())
            .await
            .map_err(|e| session_db_err(&e))?;
    }

    Ok(PreparedGatewaySession {
        session_home,
        session_home_rel,
        session_fs_label,
    })
}

async fn post_agent_feedback(
    State(state): State<AppState>,
    Json(body): Json<AgentFeedbackPostRequest>,
) -> Result<Json<AgentFeedbackPostResponse>, ApiError> {
    if body.ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let session_id = body.session_id.trim();
    if session_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId must be non-empty",
        ));
    }
    let turn = body.turn_id.trim();
    if !turn_id::validate_turn_id(turn) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "turnId must match T_<32 hex>",
        ));
    }
    validate_feedback_value(body.feedback.trim())?;
    let feedback = body.feedback.trim();
    if !state
        .session_db
        .session_exists(session_id, body.ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unknown sessionId for dsId",
        ));
    }
    if !state
        .session_db
        .turn_belongs_to_session(turn, session_id, body.ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unknown turnId for session",
        ));
    }
    let updated_at_ms = now_ms();
    state
        .session_db
        .upsert_feedback(session_id, body.ds_id, turn, feedback, updated_at_ms)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(AgentFeedbackPostResponse {
        session_id: session_id.to_string(),
        ds_id: body.ds_id,
        turn_id: turn.to_string(),
        feedback: feedback.to_string(),
        updated_at_ms,
    }))
}

async fn get_agent_feedback(
    State(state): State<AppState>,
    Query(query): Query<AgentFeedbackGetQuery>,
) -> Result<Json<AgentFeedbackGetResponse>, ApiError> {
    let Some(ds_id) = query.resolved_ds_id() else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "dsId or ds_id query parameter is required",
        ));
    };
    if ds_id < 1 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "dsId must be >= 1"));
    }
    let session_id = query.session_id.trim();
    if session_id.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "sessionId must be non-empty",
        ));
    }
    if !state
        .session_db
        .session_exists(session_id, ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
    {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown sessionId for dsId",
        ));
    }
    let items = state
        .session_db
        .list_feedback(session_id, ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(Json(AgentFeedbackGetResponse {
        session_id: session_id.to_string(),
        ds_id,
        items,
    }))
}

async fn run_solve_request(
    state: AppState,
    req: SolveRequest,
    ctx: RunSolveContext,
) -> Result<SolveResponse, ApiError> {
    validate_solve_request_fields(&req)?;
    if !ctx.skip_session_db {
        set_solve_turn_status(&state.session_db, &ctx.turn_id, "running", false).await;
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
    let project_selected = project_selected_allowed_tools(&state, req.ds_id).await?;
    let mut effective_allowed_tools = resolve_effective_allowed_tools_for_ds(
        project_selected.as_deref(),
        req.allowed_tools.as_deref(),
    )?;
    ensure_report_progress_in_allowed_tools(&mut effective_allowed_tools);

    let prepared = prepare_gateway_session(
        &state,
        req.ds_id,
        req.session_id.as_deref(),
        req.extra_session.as_ref(),
        &ctx.request_id,
        ctx.skip_session_db,
    )
    .await?;

    info!(
        target: "claw_gateway_orchestration",
        component = "solve_prepare",
        phase = "workspace_ready",
        ds_id = req.ds_id,
        request_id = %ctx.request_id,
        task_id = ctx.task_id.as_deref(),
        session_fs_id = %prepared.session_fs_label,
        session_home = %prepared.session_home.display(),
        solve_isolation = state.cfg.solve_isolation.as_str(),
        timeout_seconds,
        "session .claw/settings.json written; starting solve (container pool)"
    );

    let pool = state.docker_pool.clone();
    solve_pool::run_solve_request_docker(
        state,
        req,
        ctx,
        pool,
        started,
        effective_allowed_tools,
        solve_pool::SolveSessionPaths {
            session_home: prepared.session_home,
            session_home_rel: prepared.session_home_rel,
        },
    )
    .await
}

/// Merge MCP map for `project_config.mcp_servers_json` (solve uses DB only). Author: kejiqing
fn merge_mcp_servers_json(
    existing: &Value,
    patch: HashMap<String, Value>,
    replace: bool,
) -> Value {
    if replace {
        return Value::Object(patch.into_iter().collect());
    }
    let mut obj = existing.as_object().cloned().unwrap_or_default();
    for (k, v) in patch {
        obj.insert(k, v);
    }
    Value::Object(obj)
}

/// Upsert `project_config` MCP for a ds (`POST/DELETE /v1/mcp/inject*` write DB, not process memory).
async fn upsert_mcp_servers_for_ds(
    state: &AppState,
    ds_id: i64,
    patch: HashMap<String, Value>,
    replace: bool,
) -> Result<(), ApiError> {
    let existing = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?;
    if existing.is_some() {
        project_config_draft::ensure_draft(&state.session_db, ds_id)
            .await
            .map_err(draft_err)?;
    }
    let mut row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .unwrap_or_else(|| default_project_config_row(ds_id));
    row.mcp_servers_json = merge_mcp_servers_json(&row.mcp_servers_json, patch, replace);
    row.draft_open = true;
    row.content_rev = project_config_draft::DRAFT_CONTENT_REV.to_string();
    row.updated_at_ms = now_ms();
    state
        .session_db
        .upsert_project_config(project_config_draft::upsert_from_row(
            &row,
            project_config_draft::DRAFT_CONTENT_REV,
            row.updated_at_ms,
            row.claude_md.as_deref(),
            row.stable_content_rev.as_deref(),
        ))
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(())
}

async fn clear_mcp_servers_for_ds(
    state: &AppState,
    ds_id: i64,
    server_names: Option<Vec<String>>,
) -> Result<(), ApiError> {
    if state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .is_none()
    {
        return Ok(());
    };
    project_config_draft::ensure_draft(&state.session_db, ds_id)
        .await
        .map_err(draft_err)?;
    let mut row = state
        .session_db
        .get_project_config(ds_id)
        .await
        .map_err(|e| session_db_err(&e))?
        .expect("row exists");
    let mut obj = row
        .mcp_servers_json
        .as_object()
        .cloned()
        .unwrap_or_default();
    match server_names {
        Some(names) => {
            for name in names {
                obj.remove(&name);
            }
        }
        None => obj.clear(),
    }
    row.mcp_servers_json = Value::Object(obj);
    row.draft_open = true;
    row.content_rev = project_config_draft::DRAFT_CONTENT_REV.to_string();
    row.updated_at_ms = now_ms();
    state
        .session_db
        .upsert_project_config(project_config_draft::upsert_from_row(
            &row,
            project_config_draft::DRAFT_CONTENT_REV,
            row.updated_at_ms,
            row.claude_md.as_deref(),
            row.stable_content_rev.as_deref(),
        ))
        .await
        .map_err(|e| session_db_err(&e))?;
    Ok(())
}

fn mcp_server_names_from_settings(settings: &Value) -> Vec<String> {
    settings
        .get("mcpServers")
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
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
    upsert_mcp_servers_for_ds(&state, req.ds_id, req.mcp_servers, replace).await?;
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
    let targets = query.server_names.as_ref().map(|names| {
        names
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    });
    clear_mcp_servers_for_ds(&state, ds_id, targets).await?;
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
    let settings = {
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
        settings
    };
    let (report, loaded_names, configured_servers, status) =
        probe_mcp_load(&state.cfg.claw_bin, &work_dir, probe_timeout_seconds).await?;
    let names = mcp_server_names_from_settings(&settings);
    Ok((report, loaded_names, configured_servers, status, names))
}

/// Solve/runtime MCP: **`project_config.mcp_servers_json` only** — no `.claw.json` / env / memory fallback.
async fn build_settings(state: &AppState, ds_id: i64) -> Value {
    let mut servers = HashMap::<String, Value>::new();
    if let Ok(Some(row)) = state.session_db.get_project_config(ds_id).await {
        if let Some(extra) = row.mcp_servers_json.as_object() {
            for (k, v) in extra {
                servers.insert(k.clone(), v.clone());
            }
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

fn map_gateway_solve_turn_err(e: gateway_solve_turn::GatewaySolveTurnError) -> ApiError {
    let status = match e.status {
        504 => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
    };
    ApiError::new(status, e.message)
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

/// `1` / `true` / `yes` / `on` (case-insensitive); unset or any other value → false.
fn gateway_env_enabled(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|v| {
        let s = v.trim().to_ascii_lowercase();
        matches!(s.as_str(), "1" | "true" | "yes" | "on")
    })
}

pub(crate) fn resolve_effective_allowed_tools_for_ds(
    project_selected: Option<&[String]>,
    requested_allowed_tools: Option<&[String]>,
) -> Result<Vec<String>, ApiError> {
    project_tools::resolve_effective_allowed_tools_for_ds(
        project_selected,
        requested_allowed_tools,
    )
    .map_err(|msg| ApiError::new(StatusCode::BAD_REQUEST, msg))
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
    fn reject_deprecated_skills_sources_json() {
        assert!(reject_deprecated_skills_sources(&json!([])).is_ok());
        assert!(reject_deprecated_skills_sources(&json!([{"gitUrl": "https://x"}])).is_err());
    }

    #[test]
    fn validate_skills_json_requires_name_and_content() {
        assert!(validate_skills_json(&json!([])).is_ok());
        let ok = json!([{"skillName": "a", "skillContent": "# x"}]);
        assert!(validate_skills_json(&ok).is_ok());
        assert!(validate_skills_json(&json!([{"skillName": "a"}])).is_err());
    }

    #[allow(dead_code)]
    fn validate_skills_sources_json_requires_token_env_for_https() {
        let ok = json!([{
            "gitUrl": "https://example.com/a.git",
            "gitRef": "main",
            "tokenEnv": "CLAW_PROJECTS_GIT_TOKEN"
        }]);
        assert!(validate_skills_sources_json(&ok).is_ok());
        let missing = json!([{"gitUrl": "https://example.com/a.git", "gitRef": "main"}]);
        assert!(validate_skills_sources_json(&missing).is_err());
    }

    #[test]
    fn validate_skills_sources_json_rejects_token_in_body_and_userinfo_url() {
        let with_token = json!([{"gitUrl": "https://x.com/a.git", "token": "secret"}]);
        assert!(validate_skills_sources_json(&with_token).is_err());
        let with_userinfo = json!([{
            "gitUrl": "https://user:pass@example.com/a.git",
            "gitRef": "main"
        }]);
        assert!(validate_skills_sources_json(&with_userinfo).is_err());
        let ssh = json!([{"gitUrl": "git@github.com:org/repo.git", "gitRef": "main"}]);
        assert!(validate_skills_sources_json(&ssh).is_ok());
    }

    #[tokio::test]
    async fn ds_project_tree_ready_requires_claude_md() {
        let tmp = std::env::temp_dir().join(format!("claw-gw-ds-ready-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(!ds_project_tree_ready(&tmp).await);
        let (home_claude, _) = project_claude_paths(&tmp);
        std::fs::create_dir_all(home_claude.parent().unwrap()).unwrap();
        std::fs::write(&home_claude, "# test").unwrap();
        assert!(ds_project_tree_ready(&tmp).await);
        std::fs::write(&home_claude, "   \n").unwrap();
        assert!(!ds_project_tree_ready(&tmp).await);
        let _ = std::fs::remove_dir_all(&tmp);
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
    fn projects_git_effective_clone_url_inserts_pat_for_gitlab_https() {
        let u = projects_git_effective_clone_url(
            "https://code.sunmi.com/minidata/claw-projects-home.git",
            Some("glpat_secret"),
        );
        assert_eq!(
            u,
            "https://x-access-token:glpat_secret@code.sunmi.com/minidata/claw-projects-home.git"
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

    #[tokio::test]
    async fn task_has_report_true_when_succeeded() {
        let task = TaskRecord {
            task_id: "t1".into(),
            session_id: "t1".into(),
            request_id: "t1".into(),
            ds_id: 10,
            status: "succeeded".into(),
            created_at_ms: 0,
            started_at_ms: None,
            finished_at_ms: Some(1),
            current_task_desc: None,
            progress_updated_at_ms: None,
            result: None,
            error: None,
            turn_id: "T_00000000000000000000000000000001".into(),
            progress_history: vec![],
            has_report: false,
        };
        let url = match std::env::var("CLAW_GATEWAY_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("CLAW_GATEWAY_DATABASE_URL"))
        {
            Ok(u) => u,
            Err(_) => return,
        };
        let db = session_db::GatewaySessionDb::connect(url.trim()).await.unwrap();
        assert!(task_has_report(&db, &task).await);
    }

    #[tokio::test]
    async fn task_has_report_true_when_live_chunks_exist() {
        let url = match std::env::var("CLAW_GATEWAY_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("CLAW_GATEWAY_DATABASE_URL"))
        {
            Ok(u) => u,
            Err(_) => return,
        };
        let db = session_db::GatewaySessionDb::connect(url.trim()).await.unwrap();
        let t = now_ms();
        let sid = format!("hr_{}", uuid::Uuid::new_v4().simple());
        let turn_id = format!("T_{}", uuid::Uuid::new_v4().simple());
        db.insert_session(&sid, 1, "ds_1/sessions/hr", t).await.unwrap();
        db.insert_turn(&turn_id, &sid, 1, "running", t, None)
            .await
            .unwrap();
        db.append_live_chunks(&turn_id, &["x".into()], t)
            .await
            .unwrap();
        let task = TaskRecord {
            task_id: sid.clone(),
            session_id: sid,
            request_id: "r".into(),
            ds_id: 1,
            status: "running".into(),
            created_at_ms: t,
            started_at_ms: Some(t),
            finished_at_ms: None,
            current_task_desc: None,
            progress_updated_at_ms: None,
            result: None,
            error: None,
            turn_id,
            progress_history: vec![],
            has_report: false,
        };
        assert!(task_has_report(&db, &task).await);
        let _ = db.delete_live_chunks(&task.turn_id).await;
    }
}
