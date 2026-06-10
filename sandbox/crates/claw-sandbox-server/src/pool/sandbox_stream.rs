//! NDJSON streaming sandbox RPC (exec stdout relay). Author: kejiqing

use std::convert::Infallible;
use std::sync::Arc;

use bytes::Bytes;
use claw_sandbox_protocol::{SandboxExecChunk, SandboxRpcReq, SandboxRpcResp, TaskOutcome};
use tokio_stream::wrappers::UnboundedReceiverStream;

use super::docker_pool::DockerPoolManager;
use super::sandbox_rpc::{err_resp, ok_resp, slot_ref};

/// Exec / ExecSolve with `stream: true` (default). Author: kejiqing
#[must_use]
pub fn sandbox_rpc_wants_stream(req: &SandboxRpcReq) -> bool {
    match req {
        SandboxRpcReq::ExecSolve { stream, .. } | SandboxRpcReq::Exec { stream, .. } => *stream,
        _ => false,
    }
}

/// Spawn exec and emit NDJSON `SandboxRpcResp` lines on `tx`. Author: kejiqing
pub fn spawn_sandbox_rpc_stream(
    pool: Arc<DockerPoolManager>,
    req: SandboxRpcReq,
    tx: tokio::sync::mpsc::UnboundedSender<Result<Bytes, Infallible>>,
) {
    tokio::spawn(async move {
        let emit: Arc<dyn Fn(SandboxRpcResp) + Send + Sync> = Arc::new(move |resp| {
            if let Ok(line) = serde_json::to_string(&resp) {
                let _ = tx.send(Ok(Bytes::from(format!("{line}\n"))));
            }
        });

        let result = run_streaming_rpc(&pool, req, Arc::clone(&emit)).await;
        match result {
            Ok(outcome) => emit(ok_resp().outcome(outcome)),
            Err(resp) => emit(resp),
        }
    });
}

async fn run_streaming_rpc(
    pool: &Arc<DockerPoolManager>,
    req: SandboxRpcReq,
    emit: Arc<dyn Fn(SandboxRpcResp) + Send + Sync>,
) -> Result<TaskOutcome, SandboxRpcResp> {
    match req {
        SandboxRpcReq::ExecSolve {
            slot_index,
            task_rel,
            claw_bin,
            turn_id,
            worker_llm_env,
            ..
        } => {
            let lease = slot_ref(pool, slot_index).await.map_err(err_resp)?;
            let hook = stdout_emit_hook(emit);
            pool.exec_solve(
                &lease,
                &task_rel,
                &claw_bin,
                None,
                &turn_id,
                worker_llm_env,
                Some(hook),
            )
            .await
            .map_err(err_resp)
        }
        SandboxRpcReq::Exec {
            slot_index,
            argv,
            env,
            ..
        } => {
            let lease = slot_ref(pool, slot_index).await.map_err(err_resp)?;
            let hook = stdout_emit_hook(emit);
            pool.exec_argv(&lease, &argv, &env, Some(hook))
                .await
                .map_err(err_resp)
        }
        other => Err(err_resp(format!(
            "streaming RPC unsupported for op {:?}",
            std::mem::discriminant(&other)
        ))),
    }
}

fn stdout_emit_hook(
    emit: Arc<dyn Fn(SandboxRpcResp) + Send + Sync>,
) -> Arc<dyn Fn(String) + Send + Sync> {
    Arc::new(move |line: String| {
        emit(ok_resp().exec_chunk(SandboxExecChunk {
            kind: "stdout_line".into(),
            line: Some(line),
            exit_code: None,
        }));
    })
}

/// NDJSON byte stream for one streaming sandbox RPC request. Author: kejiqing
pub fn sandbox_rpc_ndjson_stream(
    pool: Arc<DockerPoolManager>,
    req: SandboxRpcReq,
) -> impl tokio_stream::Stream<Item = Result<Bytes, Infallible>> + Send + 'static {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    spawn_sandbox_rpc_stream(pool, req, tx);
    UnboundedReceiverStream::new(rx)
}
