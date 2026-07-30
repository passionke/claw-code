//! API error type. Author: kejiqing
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::biz_advice_report::{
    biz_report_sse_event_stream, build_biz_advice_polish_prompt, db_snapshot_report_sse_response,
    load_boss_report_writer_instructions, report_body_from_persisted,
    report_body_from_solve_output, sanitize_biz_report_parts, sanitize_external_report_text,
    sanitize_report_payload, BizAdviceReportPayload, BizReportStreamMsg, ReportExportSanitizer,
};
use crate::{
    admin_mcp_http, admin_mcp_solve, claw_tap_cluster_state, client_origin,
    gateway_admin_mcp_token, gateway_claw_tap_settings, gateway_e2b_core_readiness,
    gateway_e2b_nas_settings, gateway_e2b_observe_proxy, gateway_e2b_observe_reset,
    gateway_e2b_singleton_api, gateway_e2b_worker_settings, gateway_global_settings,
    gateway_llm_config_sync, gateway_logging, gateway_project_e2b_worker, gateway_project_llm,
    gateway_project_observe, gateway_strict_landlock_settings, gateway_translate, llm_probe,
    mcp_probe, pool, pool_consumer_resolve, preflight_plugin_api, project_config_apply,
    project_config_draft, project_config_version, project_entity_revision, project_extra_session,
    project_git_sync, project_id, project_tools, session_agent_api, session_db, session_execution,
    session_merge, session_ovs_api, session_terminal_api, session_upload, solve_pool, task_status,
    turn_id, turn_timeline_api, turn_tools_api,
};
use axum::extract::{Extension, Path as AxumPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{AppendHeaders, Html, IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use gateway_solve_turn::{
    probe_landlock, reset_task_progress, run_gateway_biz_polish_llm,
    run_gateway_biz_polish_llm_async, truncate_progress_history, ReportPolishDeepseek,
    BOSS_REPORT_SKILL_PROJ_ID,
};
use project_git_sync::{
    git_sync_list_summary, git_sync_to_json, parse_git_sync_json, GitPullOutcome,
};
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

#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub(crate) fn detail(&self) -> &str {
        &self.message
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "detail": self.message }))).into_response()
    }
}

pub(crate) fn session_routing_error(e: session_merge::SessionRoutingError) -> ApiError {
    let status = match e {
        session_merge::SessionRoutingError::AbsNotUnderWorkRoot => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        _ => StatusCode::BAD_REQUEST,
    };
    ApiError::new(status, e.detail())
}
