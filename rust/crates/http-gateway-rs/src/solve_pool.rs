//! Solve path via container pool (`docker exec claw gateway-solve-once`). Author: kejiqing

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use gateway_solve_turn::GatewaySolveTaskFile;
use tokio::fs;
use tracing::{info, warn};

use crate::pool::{parse_gateway_solve_exec_stdout, PoolOps, SlotLease};
use crate::{ApiError, AppState, RunSolveContext, SolveRequest, SolveResponse};

/// When the gateway uses [`PoolRpcClient`](crate::pool::PoolRpcClient) (TCP or Unix), session dirs
/// live under the container `CLAW_WORK_ROOT` but the host daemon must bind-mount the host path. Author: kejiqing
pub(crate) fn session_mount_for_pool_acquire(
    session_home: &Path,
    cfg: &crate::GatewayConfig,
) -> PathBuf {
    let Some(host_root) = cfg.pool_rpc_host_work_root.as_ref() else {
        return session_home.to_path_buf();
    };
    let sh = session_home.to_string_lossy();
    let wr = cfg.work_root.to_string_lossy();
    if let Some(rest) = sh.strip_prefix(wr.as_ref()) {
        let rel = rest.trim_start_matches('/');
        return host_root.join(rel);
    }
    session_home.to_path_buf()
}

/// Fixed name inside the per-session bind mount (no `..`, not client-controlled).
const GATEWAY_SOLVE_TASK_FILE: &str = "gateway-solve-task.json";

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
    started: Instant,
    effective_allowed_tools: Vec<String>,
    session_home: PathBuf,
) -> Result<SolveResponse, ApiError> {
    let RunSolveContext {
        request_id,
        task_id,
        skip_session_db: _,
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

    let session_for_pool = session_mount_for_pool_acquire(&session_home, &state.cfg);

    info!(
        target: "claw_gateway_solve_pool",
        component = "docker_solve",
        phase = "task_file_written",
        ds_id = req.ds_id,
        request_id = %request_id,
        task_id = task_id.as_deref(),
        task_path = %task_path.display(),
        session_home = %session_home.display(),
        pool_acquire_path = %session_for_pool.display(),
        task_bytes = task_bytes.len(),
        "pool solve: gateway-solve task JSON written under session dir"
    );

    let acquire_wait = Duration::from_secs(timeout_seconds.saturating_add(30));
    let lease = pool
        .acquire_slot(acquire_wait, session_for_pool.clone())
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

    if let Err(e) = pool.release_slot(lease).await {
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

#[cfg(test)]
mod session_path_tests {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use super::session_mount_for_pool_acquire;

    #[test]
    fn strips_container_work_root_for_rpc_host() {
        let cfg = crate::GatewayConfig {
            solve_isolation: crate::SolveIsolation::PodmanPool,
            claw_bin: "claw".into(),
            work_root: PathBuf::from("/var/lib/claw/workspace"),
            pool_rpc_host_work_root: Some(PathBuf::from("/host/claw/ws")),
            pool_rpc_tcp: Some("host.containers.internal:9943".into()),
            pool_rpc_unix_socket: None,
            pool_rpc_remote: true,
            ds_registry_path: Path::new("/dev/null").to_path_buf(),
            default_timeout_seconds: 1,
            default_max_iterations: 1,
            default_http_mcp_name: None,
            default_http_mcp_url: None,
            default_http_mcp_transport: "http".into(),
            config_mcp_servers: HashMap::default(),
            allowed_tools: vec![],
        };
        let got = session_mount_for_pool_acquire(
            Path::new("/var/lib/claw/workspace/ds_1/sessions/abc"),
            &cfg,
        );
        assert_eq!(got, PathBuf::from("/host/claw/ws/ds_1/sessions/abc"));
    }

    #[test]
    fn no_host_mapping_returns_session_unchanged() {
        let cfg = crate::GatewayConfig {
            solve_isolation: crate::SolveIsolation::PodmanPool,
            claw_bin: "claw".into(),
            work_root: PathBuf::from("/var/lib/claw/workspace"),
            pool_rpc_host_work_root: None,
            pool_rpc_tcp: None,
            pool_rpc_unix_socket: None,
            pool_rpc_remote: false,
            ds_registry_path: Path::new("/dev/null").to_path_buf(),
            default_timeout_seconds: 1,
            default_max_iterations: 1,
            default_http_mcp_name: None,
            default_http_mcp_url: None,
            default_http_mcp_transport: "http".into(),
            config_mcp_servers: HashMap::default(),
            allowed_tools: vec![],
        };
        let p = PathBuf::from("/tmp/sess");
        let got = session_mount_for_pool_acquire(&p, &cfg);
        assert_eq!(got, p);
    }
}
