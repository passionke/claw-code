//! Solve path via e2b sandbox (`claw gateway-solve-once`). Author: kejiqing

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use gateway_solve_turn::{otel_forward_env, GatewaySolveTaskFile};
use telemetry::{inject_traceparent, set_langfuse_trace_attrs_on_context, OtelSpanGuard};
use tokio::fs;
use tokio::time::{timeout, Duration as TokioDuration};
use tracing::{info, warn};

use crate::{ApiError, AppState, RunSolveContext, SolveRequest, SolveResponse};
use http_gateway_rs::claw_tap_cluster_state::resolve_solve_llm_route;
use http_gateway_rs::gateway_strict_landlock_settings::load_system_landlock_default;
use http_gateway_rs::pool::{
    parse_gateway_solve_exec_stdout, prepare_e2b_worker_llm_material, PoolOps,
    PrepareE2bWorkerLlmOptions, SlotLease, E2B_POOL_ID,
};

/// Map gateway container `CLAW_WORK_ROOT` paths to the host/NAS path used by e2b bind mounts.
pub(crate) fn session_mount_for_pool_acquire(
    session_home: &Path,
    cfg: &crate::GatewayConfig,
) -> PathBuf {
    crate::pool::path_for_pool_acquire(
        session_home,
        &cfg.work_root,
        cfg.pool_rpc_host_work_root.as_deref(),
    )
}

/// Fixed name inside the per-session bind mount (no `..`, not client-controlled).
const GATEWAY_SOLVE_TASK_FILE: &str = "gateway-solve-task.json";

/// Path to `claw` inside worker images. Host `CLAW_BIN` may be a macOS absolute path unusable in `podman exec`. kejiqing
const POOL_WORKER_CLAW_BIN: &str = "/usr/local/bin/claw";

fn claw_bin_for_pool_exec(cfg: &crate::GatewayConfig) -> &str {
    let bin = cfg.claw_bin.trim();
    if bin == "claw" || bin.starts_with("/usr/local/") {
        return bin;
    }
    POOL_WORKER_CLAW_BIN
}

/// Resolved session directory on disk plus path relative to `CLAW_WORK_ROOT` (same string as DB `session_home`).
pub(crate) struct SolveSessionPaths {
    pub session_home: PathBuf,
    pub session_home_rel: String,
}

/// If the async worker is aborted (e.g. `tokio::spawn` cancel), release the slot and drop the
/// cancel map entry so the pool does not leak `Leased` rows and `force_kill` can still run.
struct DockerLeaseCleanup {
    pool: Arc<dyn PoolOps + Send + Sync>,
    lease: Option<SlotLease>,
    state: AppState,
    task_id: Option<String>,
}

impl Drop for DockerLeaseCleanup {
    fn drop(&mut self) {
        let Some(lease) = self.lease.take() else {
            return;
        };
        let pool = self.pool.clone();
        let state = self.state.clone();
        let tid = self.task_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Some(ref t) = tid {
                    state.docker_slots.lock().await.remove(t);
                }
                if let Err(e) = pool.release_slot(lease).await {
                    warn!(
                        target: "claw_gateway_solve_pool",
                        component = "docker_solve",
                        phase = "lease_cleanup_release_failed",
                        error = %e,
                        "docker lease cleanup: release_slot failed after abort/cancel"
                    );
                }
            });
        } else {
            warn!(
                target: "claw_gateway_solve_pool",
                component = "docker_solve",
                phase = "lease_cleanup_no_runtime",
                "no tokio runtime; docker pool slot may leak until next warm pass"
            );
        }
    }
}

pub async fn run_solve_request_docker(
    state: AppState,
    req: SolveRequest,
    ctx: RunSolveContext,
    pool: Arc<dyn PoolOps + Send + Sync>,
    pool_id: &str,
    started: Instant,
    effective_allowed_tools: Vec<String>,
    paths: SolveSessionPaths,
) -> Result<SolveResponse, ApiError> {
    let SolveSessionPaths {
        session_home,
        session_home_rel,
    } = paths;
    let RunSolveContext {
        request_id,
        task_id,
        turn_id,
        skip_session_db: _,
        client_origin: _,
    } = ctx;
    let timeout_seconds = req
        .timeout_seconds
        .unwrap_or(state.cfg.default_timeout_seconds);

    let (llm_route, worker_llm_env) = if pool_id == E2B_POOL_ID {
        let material = prepare_e2b_worker_llm_material(
            &state.session_db,
            req.model.as_deref(),
            PrepareE2bWorkerLlmOptions { for_repl: false },
        )
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;
        (material.route, material.env)
    } else {
        resolve_solve_llm_route(
            &state.session_db,
            &state.claw_tap_cluster,
            &state.llm_runtime,
            req.model.as_deref(),
        )
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?
    };

    state
        .session_db
        .append_turn_solve_timing_bootstrap(&turn_id, "bootstrap_solve_pool_start")
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("persist bootstrap timing failed: {e}"),
            )
        })?;

    let task_path = session_home.join(GATEWAY_SOLVE_TASK_FILE);

    let session_id = req
        .session_id
        .clone()
        .or_else(|| task_id.clone())
        .unwrap_or_else(|| request_id.clone());
    let otel_guard = OtelSpanGuard::start("claw-gateway-rs", "gateway.solve", None);
    if let Some(ref g) = otel_guard {
        set_langfuse_trace_attrs_on_context(g.context(), &session_id, &turn_id, &request_id);
        g.set_attribute("langfuse.trace.name", "gateway.solve");
    }
    let otel_traceparent = otel_guard
        .as_ref()
        .and_then(|g| inject_traceparent(g.context()));
    let worker_profile = state
        .session_db
        .get_worker_profile_json(req.proj_id)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("load worker_profile_json failed: {e}"),
            )
        })?;
    let system_landlock = load_system_landlock_default(&state.session_db)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("load strictLandlockDefault failed: {e}"),
            )
        })?;
    let landlock_resolved =
        gateway_solve_turn::resolve_landlock_dsl(&worker_profile, &system_landlock)
            .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, e))?;
    let (landlock_dsl, landlock_dsl_source) = match landlock_resolved {
        Some((dsl, source)) => (Some(dsl), Some(source)),
        None => (None, None),
    };
    let task = GatewaySolveTaskFile {
        request_id: request_id.clone(),
        user_prompt: req.user_prompt.clone(),
        model: req.model.clone(),
        timeout_seconds: Some(timeout_seconds),
        extra_session: req.extra_session.clone(),
        allowed_tools: Some(effective_allowed_tools),
        max_iterations: Some(state.cfg.default_max_iterations),
        turn_id: turn_id.clone(),
        session_id: Some(session_id.clone()),
        pool_id: None,
        worker_name: None,
        llm_route: Some(serde_json::to_value(&llm_route).unwrap_or_default()),
        otel_traceparent,
        landlock_dsl,
        landlock_dsl_source,
    };
    let task_bytes = serde_json::to_vec(&task).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize gateway task failed: {e}"),
        )
    })?;
    fs::write(&task_path, &task_bytes).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write gateway task file failed: {e}"),
        )
    })?;

    let task_json = serde_json::to_value(&task).map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("serialize task json failed: {e}"),
        )
    })?;
    state
        .session_db
        .upsert_solve_task_json(&turn_id, &task_json)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("persist solve_task_json failed: {e}"),
            )
        })?;

    info!(
        target: "claw_gateway_solve_pool",
        component = "docker_solve",
        phase = "task_file_written",
        proj_id = req.proj_id,
        request_id = %request_id,
        task_id = task_id.as_deref(),
        task_path = %task_path.display(),
        session_home = %session_home.display(),
        task_bytes = task_bytes.len(),
        "pool solve: task in PG + optional gateway cache"
    );

    let acquire_wait = Duration::from_secs(timeout_seconds.saturating_add(30));
    state
        .session_db
        .append_turn_solve_timing_bootstrap(&turn_id, "bootstrap_pool_waiting")
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("persist bootstrap timing failed: {e}"),
            )
        })?;
    let lease = pool
        .acquire_slot(
            acquire_wait,
            session_id.clone(),
            req.proj_id,
            turn_id.clone(),
        )
        .await
        .map_err(|e| {
            warn!(
                target: "claw_gateway_solve_pool",
                component = "docker_solve",
                phase = "acquire_slot_failed",
                proj_id = req.proj_id,
                request_id = %request_id,
                error = %e,
                wait_secs = acquire_wait.as_secs(),
                "pool could not lease a worker (timeout or bind failure)"
            );
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e)
        })?;

    info!(
        target: "claw_gateway_solve_pool",
        component = "docker_solve",
        phase = "acquire_slot_ok",
        proj_id = req.proj_id,
        request_id = %request_id,
        slot_index = lease.slot_index,
        session_home = %session_home.display(),
        "pool slot leased; running docker exec gateway-solve-once"
    );
    state
        .session_db
        .append_turn_solve_timing_bootstrap(&turn_id, "bootstrap_pool_acquired")
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("persist bootstrap timing failed: {e}"),
            )
        })?;

    let mut lease_cleanup = DockerLeaseCleanup {
        pool: Arc::clone(&pool),
        lease: Some(lease),
        state: state.clone(),
        task_id: task_id.clone(),
    };

    if let Some(ref tid) = task_id {
        let slot_index = lease_cleanup
            .lease
            .as_ref()
            .expect("lease set after acquire")
            .slot_index;
        state
            .docker_slots
            .lock()
            .await
            .insert(tid.clone(), (Arc::clone(&pool), slot_index));
    }

    let slot_index = lease_cleanup
        .lease
        .as_ref()
        .expect("lease set for exec")
        .slot_index;
    state
        .session_db
        .append_turn_solve_timing_bootstrap(&turn_id, "bootstrap_exec_started")
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("persist bootstrap timing failed: {e}"),
            )
        })?;
    let mut exec_env = worker_llm_env;
    exec_env.extend(otel_forward_env());
    if let Some(tp) = task.otel_traceparent.as_deref() {
        exec_env.insert("TRACEPARENT".to_string(), tp.to_string());
    }
    let exec_fut = pool.exec_solve(
        lease_cleanup.lease.as_ref().expect("lease set for exec"),
        GATEWAY_SOLVE_TASK_FILE,
        claw_bin_for_pool_exec(&state.cfg),
        Some(request_id.as_str()),
        &turn_id,
        Some(exec_env),
        None,
    );
    let exec_result =
        if let Ok(outcome) = timeout(TokioDuration::from_secs(timeout_seconds), exec_fut).await {
            outcome
        } else {
            warn!(
                target: "claw_gateway_solve_pool",
                component = "docker_solve",
                phase = "exec_solve_timeout",
                proj_id = req.proj_id,
                request_id = %request_id,
                slot_index,
                timeout_seconds,
                "docker exec gateway-solve-once exceeded timeout; force-killing worker"
            );
            if let Err(e) = pool.force_kill_slot(slot_index).await {
                warn!(
                    target: "claw_gateway_solve_pool",
                    component = "docker_solve",
                    phase = "exec_solve_timeout_force_kill_failed",
                    error = %e,
                    slot_index,
                    "force_kill after solve timeout failed"
                );
            }
            Err(format!(
                "gateway-solve-once timed out after {timeout_seconds}s"
            ))
        };

    if let Some(ref tid) = task_id {
        state.docker_slots.lock().await.remove(tid);
    }
    let lease = lease_cleanup
        .lease
        .take()
        .expect("lease present until explicit handoff to release_slot");

    match &exec_result {
        Ok(outcome) => {
            if !outcome.stderr.trim().is_empty() {
                tracing::debug!(
                    target: "claw_gateway_solve_pool",
                    request_id = %request_id,
                    stderr_len = outcome.stderr.len(),
                    stderr_tail = %tail_for_log(&outcome.stderr, 2048),
                    "gateway docker exec captured stderr (see claw_gateway_solve for streamed lines)"
                );
            }
            info!(
                target: "claw_gateway_solve_pool",
                component = "docker_solve",
                phase = "exec_solve_ok",
                proj_id = req.proj_id,
                request_id = %request_id,
                slot_index = lease.slot_index,
                exit_code = outcome.exit_code,
                stdout_len = outcome.stdout.len(),
                stderr_len = outcome.stderr.len(),
                "docker exec gateway-solve-once finished"
            );
        }
        Err(e) => {
            warn!(
                target: "claw_gateway_solve_pool",
                component = "docker_solve",
                phase = "exec_solve_failed",
                proj_id = req.proj_id,
                request_id = %request_id,
                slot_index = lease.slot_index,
                error = %e,
                "docker exec gateway-solve-once failed before stdout parse"
            );
        }
    }

    // Parse before `release_slot` so bookkeeping failures never hide a good exec payload.
    let parsed = exec_result
        .as_ref()
        .ok()
        .map(|outcome| parse_gateway_solve_exec_stdout(&outcome.stdout, outcome.exit_code));

    if let Err(e) = pool.release_slot(lease).await {
        warn!(
            target: "claw_gateway_solve_pool",
            component = "docker_solve",
            phase = "release_slot_failed",
            error = %e,
            request_id = %request_id,
            proj_id = req.proj_id,
            "docker pool release_slot failed after exec (slot may recover on ensure_warm)"
        );
    }

    let _ = fs::remove_file(&task_path).await;

    exec_result.map_err(|e| {
        let status = if e.contains("timed out") {
            StatusCode::GATEWAY_TIMEOUT
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        ApiError::new(status, e)
    })?;
    let Some(parsed) = parsed else {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal: missing gateway exec parse",
        ));
    };

    let claw_exit_code = parsed.claw_exit_code;
    let output_text = parsed.output_text;
    let output_json = parsed.output_json;
    if claw_exit_code != 0 {
        let detail = output_text
            .lines()
            .map(str::trim)
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("gateway-solve-once failed");
        if let Some(ref g) = otel_guard {
            g.set_error(detail);
        }
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("gateway-solve-once failed with clawExitCode={claw_exit_code}: {detail}"),
        ));
    }

    let duration_ms = started.elapsed().as_millis() as i64;
    info!(
        target: "claw_gateway_solve_pool",
        component = "docker_solve",
        request_id = %request_id,
        task_id = task_id.as_deref().unwrap_or("-"),
        proj_id = req.proj_id,
        phase = "solve_run_ok",
        duration_ms,
        isolation = "e2b",
        claw_exit_code,
        session_home = %session_home.display(),
        "docker pool gateway_solve completed and response built"
    );
    if let Some(ref g) = otel_guard {
        g.set_ok();
    }
    Ok(SolveResponse {
        session_id: request_id.clone(),
        request_id,
        session_home_rel,
        proj_id: req.proj_id,
        work_dir: session_home.display().to_string(),
        duration_ms,
        claw_exit_code,
        output_text,
        output_json,
        turn_id,
    })
}

fn tail_for_log(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut start = s.len().saturating_sub(max_bytes);
    while start > 0 && !s.is_char_boundary(start) {
        start -= 1;
    }
    format!("…{}", &s[start..])
}

#[cfg(test)]
mod session_path_tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use super::session_mount_for_pool_acquire;

    #[test]
    fn strips_container_work_root_for_rpc_host() {
        let cfg = crate::GatewayConfig {
            claw_bin: "claw".into(),
            work_root: PathBuf::from("/var/lib/claw/workspace"),
            pool_rpc_host_work_root: Some(PathBuf::from("/host/claw/ws")),
            co_located_pool_id: Some("pool-test".into()),
            ds_registry_path: Path::new("/dev/null").to_path_buf(),
            default_timeout_seconds: 1,
            default_max_iterations: 1,
            default_http_mcp_name: None,
            default_http_mcp_url: None,
            default_http_mcp_transport: "http".into(),
            config_mcp_servers: HashMap::default(),
            projects_git_url: "git@github.com:passionke/claw-code-projects.git".into(),
            projects_git_branch: "main".into(),
            projects_git_author: "kejiqing <kejiqing@local>".into(),
            projects_git_token: None,
            projects_git_proj_home_poll_interval_secs: None,
            gateway_llm_config_poll_interval_secs: None,
            report_polish_deepseek: None,
            live_biz_report_spill_enabled: false,
        };
        let got = session_mount_for_pool_acquire(
            Path::new("/var/lib/claw/workspace/proj_1/sessions/abc"),
            &cfg,
        );
        assert_eq!(got, PathBuf::from("/host/claw/ws/proj_1/sessions/abc"));
    }

    #[test]
    fn no_host_mapping_returns_session_unchanged() {
        let cfg = crate::GatewayConfig {
            claw_bin: "claw".into(),
            work_root: PathBuf::from("/var/lib/claw/workspace"),
            pool_rpc_host_work_root: None,
            co_located_pool_id: None,
            ds_registry_path: Path::new("/dev/null").to_path_buf(),
            default_timeout_seconds: 1,
            default_max_iterations: 1,
            default_http_mcp_name: None,
            default_http_mcp_url: None,
            default_http_mcp_transport: "http".into(),
            config_mcp_servers: HashMap::default(),
            projects_git_url: "git@github.com:passionke/claw-code-projects.git".into(),
            projects_git_branch: "main".into(),
            projects_git_author: "kejiqing <kejiqing@local>".into(),
            projects_git_token: None,
            projects_git_proj_home_poll_interval_secs: None,
            gateway_llm_config_poll_interval_secs: None,
            report_polish_deepseek: None,
            live_biz_report_spill_enabled: false,
        };
        let p = PathBuf::from("/tmp/sess");
        let got = session_mount_for_pool_acquire(&p, &cfg);
        assert_eq!(got, p);
    }
}
