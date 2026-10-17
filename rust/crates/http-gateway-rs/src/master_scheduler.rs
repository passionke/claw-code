//! Lightweight daily/weekly scheduler for master solve kickoff. Author: kejiqing

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{Datelike, Local, Timelike};
use serde_json::{json, Value};
use tokio::process::Command;
use tracing::{info, warn};

use crate::admin_mcp_solve::AdminMcpSolveInput;
use crate::app_state::AppState;
use crate::master_observer::{
    render_schedule_prompt, GatewayScheduledJobRow, PROJECT_ROLE_KNOWLEDGE_BASE,
    PROJECT_ROLE_MASTER,
};
use crate::session_db::now_ms_for_registry;

const TICK_SECS: u64 = 60;

/// Spawn background ticker (one per gateway process). Author: kejiqing
pub(crate) fn spawn_master_scheduler(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(TICK_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(e) = tick_once(&state).await {
                warn!(
                    target: "claw_master_scheduler",
                    error = %e,
                    "scheduler tick failed"
                );
            }
        }
    });
}

async fn tick_once(state: &AppState) -> Result<(), String> {
    let jobs = state
        .session_db
        .list_enabled_scheduled_jobs()
        .await
        .map_err(|e| e.to_string())?;
    let now = Local::now();
    let hhmm = format!("{:02}:{:02}", now.hour(), now.minute());
    let weekday = i32::try_from(now.weekday().number_from_monday()).unwrap_or(1);
    let now_ms = now_ms_for_registry();

    for mut job in jobs {
        if !job_due(&job, &hhmm, weekday, now_ms) {
            continue;
        }
        if let Err(e) = fire_job(state, &mut job).await {
            job.last_error = Some(e.clone());
            job.updated_at_ms = now_ms;
            let _ = state.session_db.upsert_scheduled_job(&job).await;
            warn!(
                target: "claw_master_scheduler",
                job_id = %job.job_id,
                error = %e,
                "scheduled master job failed"
            );
        }
    }
    Ok(())
}

fn job_due(job: &GatewayScheduledJobRow, hhmm: &str, weekday: i32, now_ms: i64) -> bool {
    if job.run_at_hhmm.trim() != hhmm {
        return false;
    }
    if job.schedule_kind == "weekly" {
        match job.weekday {
            Some(d) if d == weekday => {}
            _ => return false,
        }
    } else if job.schedule_kind != "daily" {
        return false;
    }
    // Avoid double-fire within the same minute window (90s).
    if let Some(last) = job.last_run_at_ms {
        if now_ms.saturating_sub(last) < 90_000 {
            return false;
        }
    }
    true
}

/// Test seam for [`job_due`]. Author: kejiqing
#[cfg(test)]
pub(crate) fn job_due_for_test(
    job: &GatewayScheduledJobRow,
    hhmm: &str,
    weekday: i32,
    now_ms: i64,
) -> bool {
    job_due(job, hhmm, weekday, now_ms)
}

/// Enqueue one master solve for a scheduled job (ticker or manual run). Author: kejiqing
pub(crate) async fn fire_job(
    state: &AppState,
    job: &mut GatewayScheduledJobRow,
) -> Result<(), String> {
    let role = state
        .session_db
        .get_project_role(job.master_proj_id)
        .await
        .map_err(|e| e.to_string())?;
    if job.job_kind == "kb_sync" {
        if role != PROJECT_ROLE_KNOWLEDGE_BASE {
            return Err(format!(
                "proj {} is not knowledge_base (role={role})",
                job.master_proj_id
            ));
        }
        run_kb_sync_job(state, job).await?;
        return Ok(());
    }
    if role != PROJECT_ROLE_MASTER {
        return Err(format!(
            "proj {} is not master (role={role})",
            job.master_proj_id
        ));
    }

    // Debounce via last_run_at_ms in job_due; skip in-memory busy check (taskId==sessionId).

    let links = state
        .session_db
        .list_master_links(job.master_proj_id)
        .await
        .map_err(|e| e.to_string())?;
    let apprentice_ids: Vec<i64> = links
        .iter()
        .filter(|l| !l.orphaned)
        .map(|l| l.apprentice_proj_id)
        .collect();
    let yesterday = (Local::now() - chrono::Duration::days(1))
        .format("%Y%m%d")
        .to_string();
    let prompt = render_schedule_prompt(&job.prompt_template, &apprentice_ids, &yesterday);

    let input = AdminMcpSolveInput {
        proj_id: job.master_proj_id,
        user_prompt: prompt,
        session_id: None,
        model: None,
        timeout_seconds: None,
        extra_session: Some(json!({"bizdate": yesterday})),
        allowed_tools: None,
        max_iterations: None,
        attachments: None,
    };
    crate::admin_mcp_solve::validate_admin_mcp_solve_input(&state.session_db, &input).await?;

    // Reuse admin MCP async path via fragment helper.
    let resp = crate::routes::app::master_scheduler_enqueue_solve(state.clone(), input).await?;
    let task_id = resp
        .get("taskId")
        .or_else(|| resp.get("sessionId"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let now_ms = now_ms_for_registry();
    job.last_run_at_ms = Some(now_ms);
    job.last_task_id = task_id;
    job.last_error = None;
    job.updated_at_ms = now_ms;
    state
        .session_db
        .upsert_scheduled_job(job)
        .await
        .map_err(|e| e.to_string())?;
    info!(
        target: "claw_master_scheduler",
        job_id = %job.job_id,
        master_proj_id = job.master_proj_id,
        "enqueued scheduled master solve"
    );
    Ok(())
}

async fn run_kb_sync_job(state: &AppState, job: &mut GatewayScheduledJobRow) -> Result<(), String> {
    let cfg = state
        .session_db
        .get_project_config(job.master_proj_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("project {} has no project_config", job.master_proj_id))?;
    let kb_sources = if cfg.kb_sources_json.is_array() {
        cfg.kb_sources_json.clone()
    } else {
        return Err("kbSourcesJson must be an array".into());
    };
    let script_path = std::env::var("CLAW_KB_SYNC_SCRIPT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/app/scripts/mind-kb/apploy_faq_kb_mind_only.py".to_string());
    let gw_base =
        gateway_loopback_base().unwrap_or_else(|| state.gateway_identity.gateway_base.clone());
    let project_home = state
        .cfg
        .work_root
        .join(format!("proj_{}", job.master_proj_id))
        .join("home");
    let mcp = select_kb_sync_mcp(&cfg.mcp_servers_json)?;
    let mcp_json = serde_json::to_string(&json!({
        "url": mcp.url,
        "headers": mcp.headers,
    }))
    .map_err(|e| format!("serialize kb_sync mcp: {e}"))?;
    let mut cmd = Command::new("python3");
    cmd.arg(&script_path)
        .arg("--gw")
        .arg(gw_base)
        .arg("--faq-proj-id")
        .arg(job.master_proj_id.to_string())
        .arg("--kb-sources-json")
        .arg(kb_sources.to_string())
        .arg("--project-home")
        .arg(project_home)
        .arg("--mcp-json")
        .arg(mcp_json)
        .arg("--skip-config")
        .arg("--skip-reset");
    let output = cmd
        .output()
        .await
        .map_err(|e| format!("spawn kb_sync script: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Err(format!(
            "kb_sync script failed exit={}: {} {}",
            output.status, stdout, stderr
        ));
    }
    let now_ms = now_ms_for_registry();
    job.last_run_at_ms = Some(now_ms);
    job.last_task_id = Some(format!("kb_sync:{now_ms}"));
    job.last_error = None;
    job.updated_at_ms = now_ms;
    state
        .session_db
        .upsert_scheduled_job(job)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// HTTP MCP the gateway kb_sync job calls. Author: kejiqing
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KbSyncMcp {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

/// Pick the only `mcpServers` entry that has a non-empty `url`. Author: kejiqing
///
/// Stdio entries (no `url`) are ignored. Zero or more than one URL fails; `worker_env` is not consulted.
pub(crate) fn select_kb_sync_mcp(mcp_servers_json: &Value) -> Result<KbSyncMcp, String> {
    let obj = mcp_servers_json
        .as_object()
        .ok_or("mcpServersJson must be an object")?;
    let mut hits: Vec<KbSyncMcp> = Vec::new();
    for (name, spec) in obj {
        let Some(spec_obj) = spec.as_object() else {
            continue;
        };
        let Some(url) = spec_obj
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let mut headers = BTreeMap::new();
        if let Some(raw_headers) = spec_obj.get("headers") {
            let header_obj = raw_headers
                .as_object()
                .ok_or_else(|| format!("mcpServersJson.{name}.headers must be an object"))?;
            for (key, value) in header_obj {
                let text = value.as_str().ok_or_else(|| {
                    format!("mcpServersJson.{name}.headers.{key} must be a string")
                })?;
                headers.insert(key.clone(), text.to_string());
            }
        }
        hits.push(KbSyncMcp {
            url: url.to_string(),
            headers,
        });
    }
    match hits.len() {
        1 => Ok(hits.remove(0)),
        0 => Err("kb_sync requires exactly one MCP server with url".into()),
        n => Err(format!(
            "kb_sync requires exactly one MCP server with url, found {n}"
        )),
    }
}

fn gateway_loopback_base() -> Option<String> {
    let addr = std::env::var("CLAW_HTTP_ADDR").ok()?;
    let port = addr.rsplit(':').next()?.trim();
    if port.is_empty() {
        return None;
    }
    Some(format!("http://127.0.0.1:{port}"))
}

/// Load job by id, fire now, persist last_run / last_task / last_error. Author: kejiqing
pub(crate) async fn run_scheduled_job_now(
    state: &AppState,
    master_proj_id: i64,
    job_id: &str,
) -> Result<GatewayScheduledJobRow, String> {
    let jobs = state
        .session_db
        .list_scheduled_jobs(Some(master_proj_id))
        .await
        .map_err(|e| e.to_string())?;
    let mut job = jobs
        .into_iter()
        .find(|j| j.job_id == job_id)
        .ok_or_else(|| format!("schedule job {job_id} not found"))?;
    if job.master_proj_id != master_proj_id {
        return Err("job does not belong to this master".into());
    }
    match fire_job(state, &mut job).await {
        Ok(()) => Ok(job),
        Err(e) => {
            job.last_error = Some(e.clone());
            job.updated_at_ms = now_ms_for_registry();
            let _ = state.session_db.upsert_scheduled_job(&job).await;
            Err(e)
        }
    }
}

/// Default daily digest template. Author: kejiqing
#[must_use]
pub fn default_daily_prompt_template() -> String {
    "执行 skill master-daily-digest，学徒={{apprentice_ids}}，窗口={{bizdate_yesterday}}".into()
}

/// Default weekly repair template. Author: kejiqing
#[must_use]
pub fn default_weekly_repair_prompt_template() -> String {
    "执行 skill master-quality-repair，学徒={{apprentice_ids}}，窗口 bizdate={{bizdate_yesterday}}"
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::master_observer::render_schedule_prompt;

    fn sample_job(kind: &str, hhmm: &str, weekday: Option<i32>) -> GatewayScheduledJobRow {
        GatewayScheduledJobRow {
            job_id: "gsj_test".into(),
            master_proj_id: 1,
            job_kind: "master_digest".into(),
            schedule_kind: kind.into(),
            run_at_hhmm: hhmm.into(),
            weekday,
            enabled: true,
            prompt_template: default_daily_prompt_template(),
            last_run_at_ms: None,
            last_task_id: None,
            last_error: None,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn daily_job_due_matches_hhmm_and_debounces() {
        let job = sample_job("daily", "02:00", None);
        assert!(job_due_for_test(&job, "02:00", 1, 1_000_000));
        assert!(!job_due_for_test(&job, "02:01", 1, 1_000_000));
        let mut recent = job.clone();
        recent.last_run_at_ms = Some(1_000_000 - 30_000);
        assert!(!job_due_for_test(&recent, "02:00", 1, 1_000_000));
        recent.last_run_at_ms = Some(1_000_000 - 120_000);
        assert!(job_due_for_test(&recent, "02:00", 1, 1_000_000));
    }

    #[test]
    fn weekly_job_requires_weekday() {
        let job = sample_job("weekly", "09:30", Some(3));
        assert!(job_due_for_test(&job, "09:30", 3, 5_000_000));
        assert!(!job_due_for_test(&job, "09:30", 2, 5_000_000));
        let bad = sample_job("weekly", "09:30", None);
        assert!(!job_due_for_test(&bad, "09:30", 3, 5_000_000));
        let unknown = sample_job("monthly", "09:30", None);
        assert!(!job_due_for_test(&unknown, "09:30", 1, 5_000_000));
    }

    #[test]
    fn default_templates_reference_skills_and_placeholders() {
        let daily = default_daily_prompt_template();
        assert!(daily.contains("master-daily-digest"));
        assert!(daily.contains("{{apprentice_ids}}"));
        assert!(daily.contains("{{bizdate_yesterday}}"));
        let weekly = default_weekly_repair_prompt_template();
        assert!(weekly.contains("master-quality-repair"));
        let rendered = render_schedule_prompt(&weekly, &[7], "20260801");
        assert!(rendered.contains("7"));
        assert!(rendered.contains("20260801"));
    }

    #[test]
    fn select_kb_sync_mcp_returns_the_only_url() {
        let mcp = select_kb_sync_mcp(&json!({
            "stdio-tool": {"command": "npx"},
            "gpos-spa-mind": {
                "type": "streamable-http",
                "url": " https://mind.example/mcp ",
                "headers": {"Authorization": "Bearer mak_test"}
            }
        }))
        .unwrap();
        assert_eq!(mcp.url, "https://mind.example/mcp");
        assert_eq!(
            mcp.headers.get("Authorization").map(String::as_str),
            Some("Bearer mak_test")
        );
    }

    #[test]
    fn select_kb_sync_mcp_rejects_zero_or_many_urls() {
        let none = select_kb_sync_mcp(&json!({"stdio-tool": {"command": "npx"}})).unwrap_err();
        assert!(none.contains("exactly one"));
        let many = select_kb_sync_mcp(&json!({
            "a": {"url": "https://a.example/mcp"},
            "b": {"url": "https://b.example/mcp"}
        }))
        .unwrap_err();
        assert!(many.contains("found 2"));
    }

    #[test]
    fn select_kb_sync_mcp_ignores_worker_env_shape() {
        let err = select_kb_sync_mcp(&json!({
            "CLAW_MIND_API_URL": "https://mind.example/mcp",
            "CLAW_MIND_API_AUTHORIZATION": "Bearer ut_old"
        }))
        .unwrap_err();
        assert!(err.contains("exactly one"));
    }
}
