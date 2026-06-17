//! Pool RPC server dispatch (HTTP + line protocol). Author: kejiqing

use std::path::Path;
use std::time::Duration;

use claw_sandbox_protocol::{IsolationMode, PoolRpcReq, PoolRpcResp, SlotLease};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UnixStream};

use super::docker_pool::DockerPoolManager;

/// Shared by line-RPC handlers and HTTP `POST /v1/pool/rpc`. Author: kejiqing
#[allow(clippy::too_many_lines)]
pub async fn dispatch_pool_rpc(
    pool: &std::sync::Arc<DockerPoolManager>,
    req: PoolRpcReq,
) -> PoolRpcResp {
    match req {
        PoolRpcReq::Acquire {
            timeout_ms,
            session_id: _,
            proj_id: _,
            turn_id: _,
        } => match pool
            .acquire_slot(
                Duration::from_millis(timeout_ms),
                IsolationMode::Strict,
                None,
                None,
            )
            .await
        {
            Ok(lease) => PoolRpcResp {
                ok: true,
                error: None,
                lease: Some(lease),
                outcome: None,
                has_report: None,
                first_report_at_ms: None,
            },
            Err(e) => PoolRpcResp {
                ok: false,
                error: Some(e),
                lease: None,
                outcome: None,
                has_report: None,
                first_report_at_ms: None,
            },
        },
        PoolRpcReq::Exec {
            slot_index,
            task_rel,
            claw_bin,
            request_id,
            turn_id,
            worker_llm_env,
        } => {
            let worker_profile = match pool.worker_profile_for_slot(slot_index).await {
                Ok(p) => p,
                Err(e) => {
                    return PoolRpcResp {
                        ok: false,
                        error: Some(e),
                        lease: None,
                        outcome: None,
                        has_report: None,
                        first_report_at_ms: None,
                    };
                }
            };
            let lease = SlotLease {
                slot_index,
                worker_profile,
                worker_name: None,
                exec_identity: None,
                ttyd_host_port: None,
            };
            // Daemon path: do NOT pre-wrap with merge_stdout_hooks here. exec_solve
            // already runs merge_stdout_hooks internally (pool-local LiveReportHub).
            match pool
                .exec_solve(
                    &lease,
                    &task_rel,
                    &claw_bin,
                    request_id.as_deref(),
                    &turn_id,
                    worker_llm_env,
                    None,
                )
                .await
            {
                Ok(outcome) => PoolRpcResp {
                    ok: true,
                    error: None,
                    lease: None,
                    outcome: Some(outcome),
                    has_report: None,
                    first_report_at_ms: None,
                },
                Err(e) => PoolRpcResp {
                    ok: false,
                    error: Some(e),
                    lease: None,
                    outcome: None,
                    has_report: None,
                    first_report_at_ms: None,
                },
            }
        }
        PoolRpcReq::Release { slot_index } => {
            let worker_profile = match pool.worker_profile_for_slot(slot_index).await {
                Ok(p) => p,
                Err(e) => {
                    return PoolRpcResp {
                        ok: false,
                        error: Some(e),
                        lease: None,
                        outcome: None,
                        has_report: None,
                        first_report_at_ms: None,
                    };
                }
            };
            match pool
                .release_slot(SlotLease {
                    slot_index,
                    worker_profile,
                    worker_name: None,
                    exec_identity: None,
                    ttyd_host_port: None,
                })
                .await
            {
                Ok(()) => PoolRpcResp {
                    ok: true,
                    error: None,
                    lease: None,
                    outcome: None,
                    has_report: None,
                    first_report_at_ms: None,
                },
                Err(e) => PoolRpcResp {
                    ok: false,
                    error: Some(e),
                    lease: None,
                    outcome: None,
                    has_report: None,
                    first_report_at_ms: None,
                },
            }
        }
        PoolRpcReq::ForceKill { slot_index } => match pool.force_kill_slot(slot_index).await {
            Ok(()) => PoolRpcResp {
                ok: true,
                error: None,
                lease: None,
                outcome: None,
                has_report: None,
                first_report_at_ms: None,
            },
            Err(e) => PoolRpcResp {
                ok: false,
                error: Some(e),
                lease: None,
                outcome: None,
                has_report: None,
                first_report_at_ms: None,
            },
        },
        PoolRpcReq::ReportState { turn_id } => PoolRpcResp {
            ok: true,
            error: None,
            lease: None,
            outcome: None,
            has_report: Some(pool.has_report_for_turn(&turn_id)),
            first_report_at_ms: pool.first_report_at_ms_for_turn(&turn_id),
        },
        PoolRpcReq::SyncTurnProgress { turn_id } => {
            match pool.sync_turn_progress_to_db(&turn_id).await {
                Ok(()) => PoolRpcResp {
                    ok: true,
                    error: None,
                    lease: None,
                    outcome: None,
                    has_report: None,
                    first_report_at_ms: None,
                },
                Err(e) => PoolRpcResp {
                    ok: false,
                    error: Some(e),
                    lease: None,
                    outcome: None,
                    has_report: None,
                    first_report_at_ms: None,
                },
            }
        }
    }
}

async fn write_pool_rpc_response<W: AsyncWriteExt + Unpin>(stream: &mut W, out: &PoolRpcResp) {
    if let Ok(payload) = serde_json::to_string(out) {
        let _ = stream.write_all(format!("{payload}\n").as_bytes()).await;
    }
}

/// One Unix connection: one request line, one response line. Author: kejiqing
#[allow(dead_code)]
pub async fn handle_pool_rpc_connection(
    mut stream: UnixStream,
    pool: std::sync::Arc<DockerPoolManager>,
) {
    let mut line = String::new();
    {
        let mut reader = BufReader::new(&mut stream);
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
    }
    let Ok(req) = serde_json::from_str::<PoolRpcReq>(line.trim()) else {
        let _ = stream
            .write_all(br#"{"ok":false,"error":"invalid json"}"#)
            .await;
        let _ = stream.write_all(b"\n").await;
        return;
    };

    let out = dispatch_pool_rpc(&pool, req).await;
    write_pool_rpc_response(&mut stream, &out).await;
}

/// One TCP connection: same line protocol. Author: kejiqing
#[allow(dead_code)]
pub async fn handle_pool_rpc_tcp_connection(
    mut stream: TcpStream,
    pool: std::sync::Arc<DockerPoolManager>,
) {
    let mut line = String::new();
    {
        let mut reader = BufReader::new(&mut stream);
        if reader.read_line(&mut line).await.is_err() {
            return;
        }
    }
    let Ok(req) = serde_json::from_str::<PoolRpcReq>(line.trim()) else {
        let _ = stream
            .write_all(br#"{"ok":false,"error":"invalid json"}"#)
            .await;
        let _ = stream.write_all(b"\n").await;
        return;
    };

    let out = dispatch_pool_rpc(&pool, req).await;
    write_pool_rpc_response(&mut stream, &out).await;
}

/// Listen on Unix `path`. Author: kejiqing
#[allow(dead_code)]
pub async fn serve_pool_rpc(
    path: &Path,
    pool: std::sync::Arc<DockerPoolManager>,
) -> Result<(), String> {
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    let listener = tokio::net::UnixListener::bind(path)
        .map_err(|e| format!("bind {}: {e}", path.display()))?;
    tracing::info!(
        target: "claw_gateway_pool",
        component = "pool_daemon",
        phase = "listen_unix",
        path = %path.display(),
        "claw-pool-daemon listening (unix)"
    );
    loop {
        let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let p = std::sync::Arc::clone(&pool);
        tokio::spawn(async move {
            handle_pool_rpc_connection(stream, p).await;
        });
    }
}

/// Listen on TCP `addr` (e.g. `0.0.0.0:9943`). Author: kejiqing
#[allow(dead_code)]
pub async fn serve_pool_rpc_tcp(
    addr: &str,
    pool: std::sync::Arc<DockerPoolManager>,
) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("pool daemon tcp bind {addr}: {e}"))?;
    tracing::info!(
        target: "claw_gateway_pool",
        component = "pool_daemon",
        phase = "listen_tcp",
        addr = %addr,
        "claw-pool-daemon listening (tcp)"
    );
    loop {
        let (stream, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let p = std::sync::Arc::clone(&pool);
        tokio::spawn(async move {
            handle_pool_rpc_tcp_connection(stream, p).await;
        });
    }
}
