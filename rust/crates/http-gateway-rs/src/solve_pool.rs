//! Solve path via container pool (`docker exec claw gateway-solve-once`). Author: kejiqing

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use gateway_solve_turn::GatewaySolveTaskFile;
use tokio::fs;
use tracing::info;
use uuid::Uuid;

use crate::pool::{parse_gateway_solve_exec_stdout, DockerPoolManager, SlotLease};
use crate::{ApiError, AppState, RunSolveContext, SolveRequest, SolveResponse};

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
                    tracing::warn!(
                        error = %e,
                        "docker lease cleanup: release_slot failed after abort/cancel"
                    );
                }
            });
        } else {
            tracing::warn!("no tokio runtime; docker pool slot may leak until next warm pass");
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
) -> Result<SolveResponse, ApiError> {
    let RunSolveContext {
        request_id,
        task_id,
    } = ctx;
    let timeout_seconds = req
        .timeout_seconds
        .unwrap_or(state.cfg.default_timeout_seconds);

    let ds_home = state.cfg.work_root.join(format!("ds_{}", req.ds_id));
    let task_dir = state.cfg.work_root.join(".claw-gateway-pool-tasks");
    fs::create_dir_all(&task_dir).await.map_err(|e| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("create pool task dir failed: {e}"),
        )
    })?;
    // Task path must not incorporate client-controlled `request_id` (from `claw-session-id` /
    // `x-request-id`): path segments like `../` would escape `.claw-gateway-pool-tasks/`.
    // Author: kejiqing
    let task_file_stem = Uuid::new_v4().simple().to_string();
    let task_file_name = format!("{task_file_stem}.json");
    let task_path = task_dir.join(&task_file_name);
    let task_rel = format!(".claw-gateway-pool-tasks/{task_file_name}");

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

    let lease = pool
        .acquire_slot(Duration::from_secs(timeout_seconds.saturating_add(30)))
        .await
        .map_err(|e| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, e))?;

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
            task_rel.as_str(),
            req.ds_id,
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

    if let Ok(ref outcome) = exec_result {
        if !outcome.stderr.trim().is_empty() {
            tracing::debug!(
                request_id = %request_id,
                stderr = %outcome.stderr,
                "gateway docker exec wrote stderr"
            );
        }
    }

    // Parse before `release_slot` so bookkeeping failures never hide a good exec payload.
    let parsed = exec_result
        .as_ref()
        .ok()
        .map(|outcome| parse_gateway_solve_exec_stdout(&outcome.stdout, outcome.exit_code));

    if let Err(e) = DockerPoolManager::release_slot(&pool, lease).await {
        tracing::warn!(
            error = %e,
            request_id = %request_id,
            "docker pool release_slot failed after exec"
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
        request_id = %request_id,
        task_id = task_id.as_deref().unwrap_or("-"),
        ds_id = req.ds_id,
        phase = "solve_run_ok",
        duration_ms,
        isolation = "docker_pool",
        "gateway_solve"
    );
    Ok(SolveResponse {
        session_id: request_id.clone(),
        request_id,
        ds_id: req.ds_id,
        work_dir: ds_home.display().to_string(),
        duration_ms,
        claw_exit_code,
        output_text,
        output_json,
    })
}
