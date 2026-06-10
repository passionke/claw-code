//! End-state sandbox RPC dispatch. Author: kejiqing

use std::sync::Arc;
use std::time::Duration;

use claw_sandbox_protocol::{
    resolve_guest_path, validate_guest_abs_path, GuestExecActor, GuestFileBytes, SandboxCapacity,
    SandboxRpcReq, SandboxRpcResp, SlotLease, TaskOutcome, GUEST_LOCK_PROJECT_CONFIG_SH,
    GUEST_PREPARE_SESSION_WORKSPACE_SH,
};

use super::docker_pool::DockerPoolManager;
use super::worker_isolation::exec_user_for_actor;

pub async fn dispatch_sandbox_rpc(
    pool: &Arc<DockerPoolManager>,
    req: SandboxRpcReq,
) -> SandboxRpcResp {
    match req {
        SandboxRpcReq::Acquire {
            isolation,
            timeout_ms,
        } => match pool
            .acquire_slot(Duration::from_millis(timeout_ms), isolation)
            .await
        {
            Ok(lease) => ok_resp().lease(lease),
            Err(e) => err_resp(e),
        },
        SandboxRpcReq::GuestWipe { slot_index } => {
            let slot = match slot_ref(pool, slot_index).await {
                Ok(s) => s,
                Err(e) => return err_resp(e),
            };
            match pool.guest_wipe(&slot).await {
                Ok(()) => ok_resp().0,
                Err(e) => err_resp(e),
            }
        }
        SandboxRpcReq::GuestWrite {
            slot_index,
            volume,
            rel_path,
            bytes_b64,
        } => {
            let guest_path = match resolve_guest_path(volume, &rel_path) {
                Ok(p) => p,
                Err(e) => return err_resp(e),
            };
            let bytes = match base64_decode(&bytes_b64) {
                Ok(b) => b,
                Err(e) => return err_resp(e),
            };
            let slot = match slot_ref(pool, slot_index).await {
                Ok(s) => s,
                Err(e) => return err_resp(e),
            };
            let exec_user =
                match exec_user_for_slot(pool, slot_index, GuestExecActor::SlotWorker).await {
                    Ok(u) => u,
                    Err(e) => return err_resp(e),
                };
            match pool
                .guest_write(&slot, &guest_path, &bytes, &exec_user)
                .await
            {
                Ok(()) => ok_resp().0,
                Err(e) => err_resp(e),
            }
        }
        SandboxRpcReq::GuestExtractTarB64 {
            slot_index,
            volume,
            rel_path_prefix,
            tar_b64,
        } => {
            let prefix = match resolve_guest_path(volume, &rel_path_prefix) {
                Ok(p) => p,
                Err(e) => return err_resp(e),
            };
            let slot = match slot_ref(pool, slot_index).await {
                Ok(s) => s,
                Err(e) => return err_resp(e),
            };
            let exec_user =
                match exec_user_for_slot(pool, slot_index, GuestExecActor::SlotWorker).await {
                    Ok(u) => u,
                    Err(e) => return err_resp(e),
                };
            match pool
                .guest_extract_tar_b64(&slot, &prefix, &tar_b64, &exec_user)
                .await
            {
                Ok(()) => ok_resp().0,
                Err(e) => err_resp(e),
            }
        }
        SandboxRpcReq::GuestRead {
            slot_index,
            guest_paths,
        } => {
            for path in &guest_paths {
                if let Err(e) = validate_guest_abs_path(path) {
                    return err_resp(e);
                }
            }
            let slot = match slot_ref(pool, slot_index).await {
                Ok(s) => s,
                Err(e) => return err_resp(e),
            };
            match pool.guest_read(&slot, &guest_paths, "").await {
                Ok(files) => ok_resp().files(files),
                Err(e) => err_resp(e),
            }
        }
        SandboxRpcReq::GuestPrepareSessionWorkspace { slot_index } => {
            guest_exec_sh(
                pool,
                slot_index,
                GUEST_PREPARE_SESSION_WORKSPACE_SH,
                GuestExecActor::SlotWorker,
            )
            .await
        }
        SandboxRpcReq::GuestLockProjectConfig { slot_index } => {
            guest_exec_sh(
                pool,
                slot_index,
                GUEST_LOCK_PROJECT_CONFIG_SH,
                GuestExecActor::PoolRoot,
            )
            .await
        }
        SandboxRpcReq::GuestExecSh {
            slot_index,
            script,
            actor,
        } => guest_exec_sh(pool, slot_index, &script, actor).await,
        SandboxRpcReq::Exec {
            slot_index,
            argv,
            env,
            stream,
        } => {
            if stream {
                return err_resp(
                    "exec stream=true requires application/x-ndjson response (HTTP layer)".into(),
                );
            }
            let lease = match slot_ref(pool, slot_index).await {
                Ok(s) => s,
                Err(e) => return err_resp(e),
            };
            match pool.exec_argv(&lease, &argv, &env, None).await {
                Ok(outcome) => ok_resp().outcome(outcome),
                Err(e) => err_resp(e),
            }
        }
        SandboxRpcReq::ExecSolve {
            slot_index,
            task_rel,
            claw_bin,
            turn_id,
            worker_llm_env,
            stream,
        } => {
            if stream {
                return err_resp(
                    "exec_solve stream=true requires application/x-ndjson response (HTTP layer)"
                        .into(),
                );
            }
            let lease = match slot_ref(pool, slot_index).await {
                Ok(s) => s,
                Err(e) => return err_resp(e),
            };
            match pool
                .exec_solve(
                    &lease,
                    &task_rel,
                    &claw_bin,
                    None,
                    &turn_id,
                    worker_llm_env,
                    None,
                )
                .await
            {
                Ok(outcome) => ok_resp().outcome(outcome),
                Err(e) => err_resp(e),
            }
        }
        SandboxRpcReq::Release { slot_index } => match slot_ref(pool, slot_index).await {
            Ok(slot) => match pool.release_slot(slot).await {
                Ok(()) => ok_resp().0,
                Err(e) => err_resp(e),
            },
            Err(e) => err_resp(e),
        },
        SandboxRpcReq::ForceKill { slot_index } => match pool.force_kill_slot(slot_index).await {
            Ok(()) => ok_resp().0,
            Err(e) => err_resp(e),
        },
        SandboxRpcReq::Capacity => ok_resp().capacity(pool.capacity_async().await),
    }
}

async fn guest_exec_sh(
    pool: &Arc<DockerPoolManager>,
    slot_index: usize,
    script: &str,
    actor: GuestExecActor,
) -> SandboxRpcResp {
    let slot = match slot_ref(pool, slot_index).await {
        Ok(s) => s,
        Err(e) => return err_resp(e),
    };
    let exec_user = match exec_user_for_slot(pool, slot_index, actor).await {
        Ok(u) => u,
        Err(e) => return err_resp(e),
    };
    match pool.guest_exec_sh(&slot, script, &exec_user).await {
        Ok(()) => ok_resp().0,
        Err(e) => err_resp(e),
    }
}

async fn exec_user_for_slot(
    pool: &Arc<DockerPoolManager>,
    slot_index: usize,
    actor: GuestExecActor,
) -> Result<String, String> {
    let isolation = pool.isolation_for_slot(slot_index).await?;
    Ok(exec_user_for_actor(
        actor,
        isolation,
        pool.worker_identity(),
    ))
}

pub(crate) async fn slot_ref(pool: &DockerPoolManager, slot_index: usize) -> Result<SlotLease, String> {
    let worker_profile = pool.worker_profile_for_slot(slot_index).await?;
    Ok(SlotLease {
        slot_index,
        worker_profile,
        worker_name: None,
        exec_identity: None,
    })
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map_err(|e| format!("invalid bytes_b64: {e}"))
}

pub(crate) struct RespBuilder(SandboxRpcResp);

pub(crate) fn ok_resp() -> RespBuilder {
    RespBuilder(SandboxRpcResp {
        ok: true,
        error: None,
        lease: None,
        outcome: None,
        files: None,
        capacity: None,
        exec_chunk: None,
    })
}

pub(crate) fn err_resp(error: String) -> SandboxRpcResp {
    SandboxRpcResp {
        ok: false,
        error: Some(error),
        lease: None,
        outcome: None,
        files: None,
        capacity: None,
        exec_chunk: None,
    }
}

impl RespBuilder {
    fn lease(mut self, lease: SlotLease) -> SandboxRpcResp {
        self.0.lease = Some(lease);
        self.0
    }

    pub(crate) fn outcome(mut self, outcome: TaskOutcome) -> SandboxRpcResp {
        self.0.outcome = Some(outcome);
        self.0
    }

    fn files(mut self, files: Vec<GuestFileBytes>) -> SandboxRpcResp {
        self.0.files = Some(files);
        self.0
    }

    fn capacity(mut self, capacity: SandboxCapacity) -> SandboxRpcResp {
        self.0.capacity = Some(capacity);
        self.0
    }

    pub(crate) fn exec_chunk(
        mut self,
        chunk: claw_sandbox_protocol::SandboxExecChunk,
    ) -> SandboxRpcResp {
        self.0.exec_chunk = Some(chunk);
        self.0
    }
}

#[allow(dead_code)]
fn _task_outcome_placeholder() -> TaskOutcome {
    TaskOutcome {
        exit_code: 0,
        stdout: String::new(),
        stderr: String::new(),
    }
}
