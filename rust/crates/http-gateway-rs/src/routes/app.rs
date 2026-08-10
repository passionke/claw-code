//! Unified HTTP app handlers (include! fragments). Author: kejiqing
#![allow(
    dead_code,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::result_large_err,
    clippy::await_holding_lock,
    clippy::format_push_string,
    clippy::uninlined_format_args,
    clippy::implicit_clone,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::manual_let_else,
    clippy::match_same_arms,
    clippy::unnecessary_filter_map,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::map_unwrap_or
)]

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
    gateway_e2b_singleton_api, gateway_e2b_worker_settings, gateway_endpoint,
    gateway_global_settings, gateway_llm_config_sync, gateway_project_e2b_worker,
    gateway_project_llm, gateway_project_observe, gateway_strict_landlock_settings,
    gateway_translate, llm_probe, master_apprentice_access, master_mcp, master_observer,
    master_scheduler, mcp_probe, pool, pool_consumer_resolve, preflight_plugin_api,
    project_config_apply, project_config_draft, project_config_version, project_entity_revision,
    project_extra_session, project_git_sync, project_id, project_tools, session_agent_api,
    session_db, session_execution, session_merge, session_ovs_api, session_upload, solve_pool,
    task_status, turn_id, turn_timeline_api, turn_tools_api,
};
use axum::body::Bytes;
use axum::extract::{Extension, Path as AxumPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{AppendHeaders, Html, IntoResponse, Response};
use axum::Json;
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
use tracing::{info, warn};
use uuid::Uuid;

use crate::api_error::{session_routing_error, ApiError};
use crate::app_state::{
    container_runtime_bin, AppState, GatewayConfig, HttpRequestId, PreparedGatewaySession,
    RunSolveContext, SolveAsyncResponse, SolveRequest, SolveResponse, SolveStartResponse,
    StartRequest, TaskInner, TaskRecord,
};

include!("fragments/shared.rs");
include!("fragments/health.rs");
include!("fragments/meta.rs");
include!("fragments/projects.rs");
include!("fragments/project_config.rs");
include!("fragments/project_assets.rs");
include!("fragments/project_inference.rs");
include!("fragments/gateway_settings.rs");
include!("fragments/admin_mcp.rs");
include!("fragments/master.rs");
include!("fragments/sessions.rs");
include!("fragments/turns.rs");
include!("fragments/solve.rs");
include!("fragments/tasks.rs");
include!("fragments/biz_report.rs");
include!("fragments/mcp.rs");
include!("fragments/pools.rs");

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
    async fn proj_tree_ready_requires_claude_md_or_applied_empty_override() {
        let tmp = std::env::temp_dir().join(format!("claw-gw-ds-ready-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(!proj_tree_ready(&tmp, None).await);
        let (home_claude, _) = project_claude_paths(&tmp);
        std::fs::create_dir_all(home_claude.parent().unwrap()).unwrap();
        std::fs::write(&home_claude, "# test").unwrap();
        assert!(proj_tree_ready(&tmp, None).await);
        std::fs::write(&home_claude, "   \n").unwrap();
        assert!(!proj_tree_ready(&tmp, None).await);

        let row = session_db::ProjectConfigRow {
            proj_id: 1,
            content_rev: "rev-ready".into(),
            stable_content_rev: Some("rev-ready".into()),
            draft_open: false,
            updated_at_ms: 0,
            rules_json: json!([]),
            mcp_servers_json: json!({}),
            skills_sources_json: json!([]),
            skills_json: json!([]),
            allowed_tools_json: json!([]),
            claude_md: None,
            git_sync_json: json!({}),
            solve_preflight_json: json!({"kind": "none"}),
            solve_orchestration_json: json!({"kind": "single_turn"}),
            language_pipeline_json: json!({}),
            extra_session_fields_json: json!([]),
            prompt_limits_json: json!({}),
            worker_profile_json: json!({"mode": "strict"}),
            worker_env_json: json!({}),
            project_code: String::new(),
            project_description: String::new(),
            max_iterations: None,
        };
        std::fs::create_dir_all(tmp.join(".claw")).unwrap();
        std::fs::write(
            tmp.join(project_config_apply::APPLIED_REV_MARKER),
            "rev-ready",
        )
        .unwrap();
        assert!(proj_tree_ready(&tmp, Some(&row)).await);
        std::fs::write(&home_claude, "stale after clear").unwrap();
        assert!(
            !proj_tree_ready(&tmp, Some(&row)).await,
            "stale CLAUDE on disk must block ready when claude_md is empty"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn proj_work_dir_and_claude_paths_match_contract() {
        let root = Path::new("/tmp/gateway-work");
        let work_dir = proj_work_dir(root, 27);
        assert_eq!(work_dir, PathBuf::from("/tmp/gateway-work/proj_27"));
        let (home_claude, root_claude) = project_claude_paths(&work_dir);
        assert_eq!(
            home_claude,
            PathBuf::from("/tmp/gateway-work/proj_27/home/CLAUDE.md")
        );
        assert_eq!(
            root_claude,
            PathBuf::from("/tmp/gateway-work/proj_27/CLAUDE.md")
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

    #[test]
    fn task_has_report_contract_pool_sse_mode() {
        for (status, want) in [
            ("queued", false),
            ("running", false),
            ("succeeded", true),
            ("failed", false),
        ] {
            let got = task_has_report_for_status(status, false);
            assert_eq!(got, want, "status={status}");
        }
    }

    #[test]
    fn task_has_report_contract_legacy_spill_mode() {
        for (status, want) in [
            ("queued", false),
            ("running", false),
            ("succeeded", true),
            ("failed", false),
        ] {
            let got = task_has_report_for_status(status, true);
            assert_eq!(got, want, "status={status}");
        }
    }
}
