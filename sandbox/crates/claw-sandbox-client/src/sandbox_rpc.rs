//! HTTP client for end-state sandbox RPC. Author: kejiqing

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use claw_sandbox_protocol::{
    GuestExecActor, GuestVolume, IsolationMode, SandboxRpcReq, SandboxRpcResp, SlotLease,
    TaskOutcome,
};
use futures_util::StreamExt;

const NDJSON_MIME: &str = "application/x-ndjson";

#[derive(Debug, Clone)]
pub struct SandboxRpcClient {
    base_url: String,
    http: reqwest::Client,
}

impl SandboxRpcClient {
    #[must_use]
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .build()
                .expect("sandbox rpc http client"),
        }
    }

    pub async fn call(&self, req: SandboxRpcReq) -> Result<SandboxRpcResp, String> {
        let url = format!("{}/v1/sandbox/rpc", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("sandbox rpc POST {url}: {e}"))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("sandbox rpc read body: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "sandbox rpc {url} status {status}: {}",
                body.chars().take(500).collect::<String>()
            ));
        }
        serde_json::from_str(&body).map_err(|e| format!("sandbox rpc decode: {e}: {body}"))
    }

    async fn call_exec_stream(
        &self,
        req: SandboxRpcReq,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TaskOutcome, String> {
        let url = format!("{}/v1/sandbox/rpc", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("sandbox rpc POST {url}: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "sandbox rpc {url} status {status}: {}",
                body.chars().take(500).collect::<String>()
            ));
        }
        let ctype = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !ctype.contains(NDJSON_MIME) {
            let body = resp
                .text()
                .await
                .map_err(|e| format!("sandbox rpc read body: {e}"))?;
            let parsed: SandboxRpcResp = serde_json::from_str(&body)
                .map_err(|e| format!("sandbox rpc decode: {e}: {body}"))?;
            if !parsed.ok {
                return Err(parsed.error.unwrap_or_else(|| "sandbox exec failed".into()));
            }
            return parsed
                .outcome
                .ok_or_else(|| "sandbox exec: missing outcome".into());
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        let mut outcome: Option<TaskOutcome> = None;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("sandbox rpc stream read: {e}"))?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find('\n') {
                let line = buf[..pos].trim().to_string();
                buf.drain(..=pos);
                if line.is_empty() {
                    continue;
                }
                let parsed: SandboxRpcResp = serde_json::from_str(&line)
                    .map_err(|e| format!("sandbox rpc ndjson decode: {e}: {line}"))?;
                if !parsed.ok {
                    return Err(parsed
                        .error
                        .unwrap_or_else(|| "sandbox exec stream failed".into()));
                }
                if let Some(chunk) = parsed.exec_chunk {
                    if chunk.kind == "stdout_line" {
                        if let Some(line) = chunk.line {
                            if let Some(ref hook) = on_stdout_line {
                                hook(line);
                            }
                        }
                    }
                }
                if let Some(o) = parsed.outcome {
                    outcome = Some(o);
                }
            }
        }
        outcome.ok_or_else(|| "sandbox exec stream: missing outcome".into())
    }

    pub async fn acquire(
        &self,
        wait: Duration,
        isolation: IsolationMode,
    ) -> Result<SlotLease, String> {
        let r = self
            .call(SandboxRpcReq::Acquire {
                isolation,
                timeout_ms: u64::try_from(wait.as_millis()).unwrap_or(u64::MAX),
            })
            .await?;
        if !r.ok {
            return Err(r.error.unwrap_or_else(|| "acquire failed".into()));
        }
        r.lease.ok_or_else(|| "acquire: missing lease".into())
    }

    pub async fn guest_wipe(&self, slot_index: usize) -> Result<(), String> {
        let r = self.call(SandboxRpcReq::GuestWipe { slot_index }).await?;
        if r.ok {
            Ok(())
        } else {
            Err(r.error.unwrap_or_else(|| "guest_wipe failed".into()))
        }
    }

    pub async fn guest_write(
        &self,
        slot_index: usize,
        volume: GuestVolume,
        rel_path: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        use base64::Engine;
        let bytes_b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        let r = self
            .call(SandboxRpcReq::GuestWrite {
                slot_index,
                volume,
                rel_path: rel_path.to_string(),
                bytes_b64,
            })
            .await?;
        if r.ok {
            Ok(())
        } else {
            Err(r.error.unwrap_or_else(|| "guest_write failed".into()))
        }
    }

    pub async fn guest_extract_tar_b64(
        &self,
        slot_index: usize,
        volume: GuestVolume,
        rel_path_prefix: &str,
        tar_b64: &str,
    ) -> Result<(), String> {
        let r = self
            .call(SandboxRpcReq::GuestExtractTarB64 {
                slot_index,
                volume,
                rel_path_prefix: rel_path_prefix.to_string(),
                tar_b64: tar_b64.to_string(),
            })
            .await?;
        if r.ok {
            Ok(())
        } else {
            Err(r.error.unwrap_or_else(|| "guest_extract_tar failed".into()))
        }
    }

    pub async fn guest_read(
        &self,
        slot_index: usize,
        guest_paths: &[String],
    ) -> Result<Vec<(String, Vec<u8>)>, String> {
        use base64::Engine;
        let r = self
            .call(SandboxRpcReq::GuestRead {
                slot_index,
                guest_paths: guest_paths.to_vec(),
            })
            .await?;
        if !r.ok {
            return Err(r.error.unwrap_or_else(|| "guest_read failed".into()));
        }
        let files = r.files.unwrap_or_default();
        files
            .into_iter()
            .map(|f| {
                let path = f.path;
                base64::engine::general_purpose::STANDARD
                    .decode(f.bytes_b64.trim())
                    .map(|b| (path.clone(), b))
                    .map_err(|e| format!("guest_read decode {path}: {e}"))
            })
            .collect()
    }

    pub async fn guest_prepare_session_workspace(&self, slot_index: usize) -> Result<(), String> {
        let r = self
            .call(SandboxRpcReq::GuestPrepareSessionWorkspace { slot_index })
            .await?;
        if r.ok {
            Ok(())
        } else {
            Err(r
                .error
                .unwrap_or_else(|| "guest_prepare_session_workspace failed".into()))
        }
    }

    pub async fn guest_lock_project_config(&self, slot_index: usize) -> Result<(), String> {
        let r = self
            .call(SandboxRpcReq::GuestLockProjectConfig { slot_index })
            .await?;
        if r.ok {
            Ok(())
        } else {
            Err(r
                .error
                .unwrap_or_else(|| "guest_lock_project_config failed".into()))
        }
    }

    pub async fn guest_exec_sh(
        &self,
        slot_index: usize,
        script: &str,
        actor: GuestExecActor,
    ) -> Result<(), String> {
        let r = self
            .call(SandboxRpcReq::GuestExecSh {
                slot_index,
                script: script.to_string(),
                actor,
            })
            .await?;
        if r.ok {
            Ok(())
        } else {
            Err(r.error.unwrap_or_else(|| "guest_exec_sh failed".into()))
        }
    }

    pub async fn exec_solve(
        &self,
        slot_index: usize,
        task_rel: &str,
        claw_bin: &str,
        turn_id: &str,
        worker_llm_env: Option<BTreeMap<String, String>>,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TaskOutcome, String> {
        self.call_exec_stream(
            SandboxRpcReq::ExecSolve {
                slot_index,
                task_rel: task_rel.to_string(),
                claw_bin: claw_bin.to_string(),
                turn_id: turn_id.to_string(),
                worker_llm_env,
                stream: true,
            },
            on_stdout_line,
        )
        .await
    }

    pub async fn release(&self, slot_index: usize) -> Result<(), String> {
        let r = self.call(SandboxRpcReq::Release { slot_index }).await?;
        if r.ok {
            Ok(())
        } else {
            Err(r.error.unwrap_or_else(|| "release failed".into()))
        }
    }

    pub async fn force_kill(&self, slot_index: usize) -> Result<(), String> {
        let r = self.call(SandboxRpcReq::ForceKill { slot_index }).await?;
        if r.ok {
            Ok(())
        } else {
            Err(r.error.unwrap_or_else(|| "force_kill failed".into()))
        }
    }
}
