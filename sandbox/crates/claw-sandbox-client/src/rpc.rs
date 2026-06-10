//! Line-delimited JSON pool RPC: TCP (`host:port`) or Unix path. Default deploy uses TCP from gateway container to host daemon. Author: kejiqing

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UnixStream};

use claw_sandbox_protocol::{PoolRpcReq, PoolRpcResp, SlotLease, TaskOutcome};

use super::traits::PoolOps;

#[derive(Debug, Clone)]
enum PoolRpcTransport {
    Unix(PathBuf),
    /// `host:port`, e.g. `host.containers.internal:9943`.
    Tcp(String),
    /// `http://host:9944` — same server as pool live-report HTTP. Author: kejiqing
    Http(String),
}

/// Client for host `claw-pool-daemon` / `claw-sandbox` (TCP or Unix). Author: kejiqing
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

    /// Pool daemon HTTP base, e.g. `http://host.containers.internal:9944`. Author: kejiqing
    #[must_use]
    pub fn new_http(base_url: &str) -> Self {
        Self {
            transport: PoolRpcTransport::Http(base_url.trim().trim_end_matches('/').to_string()),
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
            PoolRpcTransport::Http(base) => {
                let url = format!("{base}/v1/pool/rpc");
                let client = reqwest::Client::builder()
                    .connect_timeout(Duration::from_secs(15))
                    .build()
                    .map_err(|e| format!("pool rpc http client: {e}"))?;
                let resp = client
                    .post(&url)
                    .json(&req)
                    .send()
                    .await
                    .map_err(|e| format!("pool rpc http POST {url}: {e}"))?;
                let status = resp.status();
                let body = resp
                    .text()
                    .await
                    .map_err(|e| format!("pool rpc http read body: {e}"))?;
                if !status.is_success() {
                    return Err(format!(
                        "pool rpc http {url} status {status}: {}",
                        body.chars().take(500).collect::<String>()
                    ));
                }
                body
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
        session_id: String,
        proj_id: i64,
        turn_id: String,
    ) -> Result<SlotLease, String> {
        let r = self
            .call(PoolRpcReq::Acquire {
                timeout_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
                session_id,
                proj_id,
                turn_id,
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
        turn_id: &str,
        worker_llm_env: Option<BTreeMap<String, String>>,
        _on_stdout_line: Option<std::sync::Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TaskOutcome, String> {
        let r = self
            .call(PoolRpcReq::Exec {
                slot_index: slot.slot_index,
                task_rel: task_rel_under_root.to_string(),
                claw_bin: claw_bin.to_string(),
                request_id: request_id.map(str::to_string),
                turn_id: turn_id.to_string(),
                worker_llm_env,
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

    async fn has_report_for_turn(&self, turn_id: &str) -> bool {
        self.call(PoolRpcReq::ReportState {
            turn_id: turn_id.to_string(),
        })
        .await
        .ok()
        .and_then(|r| r.has_report)
        .unwrap_or(false)
    }

    async fn first_report_at_ms_for_turn(&self, turn_id: &str) -> Option<i64> {
        self.call(PoolRpcReq::ReportState {
            turn_id: turn_id.to_string(),
        })
        .await
        .ok()
        .and_then(|r| r.first_report_at_ms)
    }

    async fn sync_turn_progress_to_db(&self, turn_id: &str) -> Result<(), String> {
        let r = self
            .call(PoolRpcReq::SyncTurnProgress {
                turn_id: turn_id.to_string(),
            })
            .await?;
        if !r.ok {
            return Err(r
                .error
                .unwrap_or_else(|| "pool sync_turn_progress failed".into()));
        }
        Ok(())
    }
}
