//! Solve path via container pool (`docker exec claw gateway-solve-once`). Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use gateway_solve_turn::GatewaySolveTaskFile;
use tokio::fs;
use tracing::{info, warn};

use crate::pool::{parse_gateway_solve_exec_stdout, DockerPoolManager, SlotLease};
use crate::{ApiError, AppState, RunSolveContext, SolveRequest, SolveResponse};

/// Fixed name inside the per-session bind mount (no `..`, not client-controlled).
const GATEWAY_SOLVE_TASK_FILE: &str = "gateway-solve-task.json";

/// If the async worker is aborted (e.g. `tokio::spawn` cancel), release the slot and drop the
/// cancel map entry so the pool does not leak `Leased` rows and `force_kill` can still run.
struct DockerLeaseCleanup {
    pool: Arc<DockerPoolManager>,
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
                if let Err(e) = DockerPoolManager::release_slot(&pool, lease).await {
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
    pool: Arc<DockerPoolManager>,
    started: Instant,
    effective_allowed_tools: Vec<String>,
    session_home: PathBuf,
) -> Result<SolveResponse, ApiError> {
    let RunSolveContext {
        request_id,
        task_id,
    } = ctx;
    let timeout_seconds = req
        .timeout_seconds
        .unwrap_or(state.cfg.default_timeout_seconds);

    let task_path = session_home.join(GATEWAY_SOLVE_TASK_FILE);

    let task = GatewaySolveTaskFile {
        request_id: request_id.clone(),
        user_prompt: req.user_prompt.clone(),
        model: req.model.clone(),
        timeout_seconds: Some(timeout_seconds),
        extra_session: req.extra_session.clone(),
        allowed_tools: Some(effective_allowed_tools),
        max_iterations: Some(state.cfg.default_max_iterations),
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

    info!(
        target: "claw_gateway_solve_pool",
        component = "docker_solve",
        phase = "task_file_written",
        ds_id = req.ds_id,
        request_id = %request_id,
        task_id = task_id.as_deref(),
        task_path = %task_path.display(),
        session_home = %session_home.display(),
        task_bytes = task_bytes.len(),
        "pool solve: gateway-solve task JSON written under session dir"
    );

    let acquire_wait = Duration::from_secs(timeout_seconds.saturating_add(30));
    let lease = pool
        .acquire_slot(acquire_wait, session_home.clone())
        .await
        .map_err(|e| {
            warn!(
                target: "claw_gateway_solve_pool",
                component = "docker_solve",
                phase = "acquire_slot_failed",
                ds_id = req.ds_id,
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
        ds_id = req.ds_id,
        request_id = %request_id,
        slot_index = lease.slot_index,
        session_home = %session_home.display(),
        "pool slot leased; running docker exec gateway-solve-once"
    );

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

    let exec_result = pool
        .exec_solve(
            lease_cleanup.lease.as_ref().expect("lease set for exec"),
            GATEWAY_SOLVE_TASK_FILE,
            state.cfg.claw_bin.as_str(),
            Some(request_id.as_str()),
        )
        .await;

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
                ds_id = req.ds_id,
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
                ds_id = req.ds_id,
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

    if let Err(e) = DockerPoolManager::release_slot(&pool, lease).await {
        warn!(
            target: "claw_gateway_solve_pool",
            component = "docker_solve",
            phase = "release_slot_failed",
            error = %e,
            request_id = %request_id,
            ds_id = req.ds_id,
            "docker pool release_slot failed after exec (slot may recover on ensure_warm)"
        );
    }

    let _ = fs::remove_file(&task_path).await;

    exec_result.map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let Some(parsed) = parsed else {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal: missing gateway exec parse",
        ));
    };

    let claw_exit_code = parsed.claw_exit_code;
    let output_text = parsed.output_text;
    let output_json = parsed.output_json;

    let duration_ms = started.elapsed().as_millis() as i64;
    info!(
        target: "claw_gateway_solve_pool",
        component = "docker_solve",
        request_id = %request_id,
        task_id = task_id.as_deref().unwrap_or("-"),
        ds_id = req.ds_id,
        phase = "solve_run_ok",
        duration_ms,
        isolation = "docker_pool",
        claw_exit_code,
        session_home = %session_home.display(),
        "docker pool gateway_solve completed and response built"
    );
    Ok(SolveResponse {
        session_id: request_id.clone(),
        request_id,
        ds_id: req.ds_id,
        work_dir: session_home.display().to_string(),
        duration_ms,
        claw_exit_code,
        output_text,
        output_json,
    })
}

fn tail_for_log(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let start = s.floor_char_boundary(s.len().saturating_sub(max_bytes));
    format!("…{}", &s[start..])
}
