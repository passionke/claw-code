//! Line-delimited JSON pool RPC: TCP (`host:port`) or Unix path. Default deploy uses TCP from gateway container to host daemon. Author: kejiqing

use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UnixStream};

use super::docker_pool::DockerPoolManager;
use super::traits::{PoolOps, PoolSessionHostMounts, SlotLease, TaskOutcome};

#[derive(Debug, Clone)]
enum PoolRpcTransport {
    Unix(PathBuf),
    /// `host:port`, e.g. `host.containers.internal:9943`.
    Tcp(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PoolRpcReq {
    Acquire {
        timeout_ms: u64,
        session_host_mount: String,
        #[serde(default)]
        skills_host_mount: Option<String>,
        /// Host path to `ds_*/CLAUDE.md` for optional ro file bind (gateway ≥ pool daemon that understands this field).
        #[serde(default)]
        claude_md_host_mount: Option<String>,
    },
    Exec {
        slot_index: usize,
        task_rel: String,
        claw_bin: String,
        request_id: Option<String>,
    },
    Release {
        slot_index: usize,
    },
    ForceKill {
        slot_index: usize,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PoolRpcResp {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<SlotLease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TaskOutcome>,
}

/// Client for host `claw-pool-daemon` (TCP or Unix). Author: kejiqing
#[derive(Debug, Clone)]
pub struct PoolRpcClient {
    transport: PoolRpcTransport,
}

impl PoolRpcClient {
    /// Unix domain path (legacy / Linux-friendly).
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            transport: PoolRpcTransport::Unix(path),
        }
    }

    #[must_use]
    pub fn new_tcp(host_port: String) -> Self {
        Self {
            transport: PoolRpcTransport::Tcp(host_port),
        }
    }

    async fn call(&self, req: PoolRpcReq) -> Result<PoolRpcResp, String> {
        let payload = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        let line = match &self.transport {
            PoolRpcTransport::Unix(path) => {
                let mut stream = UnixStream::connect(path)
                    .await
                    .map_err(|e| format!("pool rpc connect {}: {e}", path.display()))?;
                stream
                    .write_all(format!("{payload}\n").as_bytes())
                    .await
                    .map_err(|e| format!("pool rpc write: {e}"))?;
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader
                    .read_line(&mut line)
                    .await
                    .map_err(|e| format!("pool rpc read: {e}"))?;
                line
            }
            PoolRpcTransport::Tcp(addr) => {
                let mut stream = TcpStream::connect(addr)
                    .await
                    .map_err(|e| format!("pool rpc tcp connect {addr}: {e}"))?;
                stream
                    .write_all(format!("{payload}\n").as_bytes())
                    .await
                    .map_err(|e| format!("pool rpc write: {e}"))?;
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader
                    .read_line(&mut line)
                    .await
                    .map_err(|e| format!("pool rpc read: {e}"))?;
                line
            }
        };
        serde_json::from_str::<PoolRpcResp>(line.trim())
            .map_err(|e| format!("pool rpc decode: {e}: {line}"))
    }
}

#[async_trait]
impl PoolOps for PoolRpcClient {
    async fn acquire_slot(
        &self,
        wait: Duration,
        session_host_mount: PathBuf,
        host_mounts: PoolSessionHostMounts,
    ) -> Result<SlotLease, String> {
        let r = self
            .call(PoolRpcReq::Acquire {
                timeout_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                session_host_mount: session_host_mount.to_string_lossy().into_owned(),
                skills_host_mount: host_mounts
                    .skills_dir
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
                claude_md_host_mount: host_mounts
                    .claude_md_file
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned()),
            })
            .await?;
        if !r.ok {
            return Err(r.error.unwrap_or_else(|| "pool acquire failed".into()));
        }
        r.lease.ok_or_else(|| "pool acquire: missing lease".into())
    }

    async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        request_id: Option<&str>,
    ) -> Result<TaskOutcome, String> {
        let r = self
            .call(PoolRpcReq::Exec {
                slot_index: slot.slot_index,
                task_rel: task_rel_under_root.to_string(),
                claw_bin: claw_bin.to_string(),
                request_id: request_id.map(str::to_string),
            })
            .await?;
        if !r.ok {
            return Err(r.error.unwrap_or_else(|| "pool exec failed".into()));
        }
        r.outcome.ok_or_else(|| "pool exec: missing outcome".into())
    }

    async fn release_slot(&self, slot: SlotLease) -> Result<(), String> {
        let r = self
            .call(PoolRpcReq::Release {
                slot_index: slot.slot_index,
            })
            .await?;
        if !r.ok {
            return Err(r.error.unwrap_or_else(|| "pool release failed".into()));
        }
        Ok(())
    }

    async fn force_kill_slot(&self, slot_index: usize) -> Result<(), String> {
        let r = self.call(PoolRpcReq::ForceKill { slot_index }).await?;
        if !r.ok {
            return Err(r.error.unwrap_or_else(|| "pool force_kill failed".into()));
        }
        Ok(())
    }
}

async fn dispatch_pool_rpc(
    pool: &std::sync::Arc<DockerPoolManager>,
    req: PoolRpcReq,
) -> PoolRpcResp {
    match req {
        PoolRpcReq::Acquire {
            timeout_ms,
            session_host_mount,
            skills_host_mount,
            claude_md_host_mount,
        } => match pool
            .acquire_slot(
                Duration::from_millis(timeout_ms),
                PathBuf::from(session_host_mount),
                PoolSessionHostMounts {
                    skills_dir: skills_host_mount.map(PathBuf::from),
                    claude_md_file: claude_md_host_mount.map(PathBuf::from),
                },
            )
            .await
        {
            Ok(lease) => PoolRpcResp {
                ok: true,
                error: None,
                lease: Some(lease),
                outcome: None,
            },
            Err(e) => PoolRpcResp {
                ok: false,
                error: Some(e),
                lease: None,
                outcome: None,
            },
        },
        PoolRpcReq::Exec {
            slot_index,
            task_rel,
            claw_bin,
            request_id,
        } => {
            let lease = SlotLease { slot_index };
            match pool
                .exec_solve(&lease, &task_rel, &claw_bin, request_id.as_deref())
                .await
            {
                Ok(outcome) => PoolRpcResp {
                    ok: true,
                    error: None,
                    lease: None,
                    outcome: Some(outcome),
                },
                Err(e) => PoolRpcResp {
                    ok: false,
                    error: Some(e),
                    lease: None,
                    outcome: None,
                },
            }
        }
        PoolRpcReq::Release { slot_index } => {
            match pool.release_slot(SlotLease { slot_index }).await {
                Ok(()) => PoolRpcResp {
                    ok: true,
                    error: None,
                    lease: None,
                    outcome: None,
                },
                Err(e) => PoolRpcResp {
                    ok: false,
                    error: Some(e),
                    lease: None,
                    outcome: None,
                },
            }
        }
        PoolRpcReq::ForceKill { slot_index } => match pool.force_kill_slot(slot_index).await {
            Ok(()) => PoolRpcResp {
                ok: true,
                error: None,
                lease: None,
                outcome: None,
            },
            Err(e) => PoolRpcResp {
                ok: false,
                error: Some(e),
                lease: None,
                outcome: None,
            },
        },
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
