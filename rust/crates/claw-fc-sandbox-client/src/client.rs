//! E2B-compatible REST client for Alibaba FC cloud sandbox. Author: kejiqing

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tracing::{debug, warn};

/// Renew sandbox TTL when local estimate of remaining time falls below this (e2b min is 5m).
pub const SANDBOX_LEASE_RENEW_LEAD_SECS: u64 = 300;

use crate::config::FcSandboxConfig;
use crate::types::{CreateSandboxResponse, FcExecOutcome, FcSandboxHandle, FcSandboxVolumeMount};

enum FcExecHelperIngest {
    More,
    Outcome(FcExecOutcome),
    Error(String),
}

fn ingest_fc_exec_helper_line(
    parsed: &Value,
    trimmed: &str,
    on_stdout_line: Option<&Arc<dyn Fn(String) + Send + Sync>>,
) -> FcExecHelperIngest {
    if parsed.get("ev").and_then(Value::as_str) == Some("stdout_line") {
        if let Some(chunk) = parsed.get("line").and_then(Value::as_str) {
            if let Some(hook) = on_stdout_line {
                hook(chunk.to_string());
            }
        }
        return FcExecHelperIngest::More;
    }
    if parsed.get("ok").and_then(Value::as_bool) == Some(false) {
        let err = parsed
            .get("error")
            .and_then(Value::as_str)
            .map_or_else(|| trimmed.to_string(), str::to_string);
        return FcExecHelperIngest::Error(err);
    }
    if parsed.get("ok").and_then(Value::as_bool) == Some(true) {
        if let Some(exit_code) = parsed.get("exit_code").and_then(Value::as_i64) {
            return FcExecHelperIngest::Outcome(FcExecOutcome {
                exit_code: i32::try_from(exit_code).unwrap_or(-1),
                stdout: parsed
                    .get("stdout")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                stderr: parsed
                    .get("stderr")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
        return FcExecHelperIngest::Outcome(FcExecOutcome {
            exit_code: 0,
            stdout: parsed
                .get("stdout")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            stderr: String::new(),
        });
    }
    FcExecHelperIngest::More
}

/// HTTP client for FC sandbox lifecycle + delegated envd exec.
#[derive(Debug, Clone)]
pub struct FcSandboxClient {
    config: FcSandboxConfig,
    http: reqwest::Client,
    /// Local TTL estimate per sandbox (`POST /timeout` resets from request time).
    lease_expires: Arc<Mutex<HashMap<String, Instant>>>,
}

impl FcSandboxClient {
    #[must_use]
    pub fn new(config: FcSandboxConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            lease_expires: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// True when remaining TTL is below [`SANDBOX_LEASE_RENEW_LEAD_SECS`].
    #[must_use]
    pub fn lease_should_renew(remaining_secs: u64) -> bool {
        remaining_secs < SANDBOX_LEASE_RENEW_LEAD_SECS
    }

    fn register_sandbox_lease(&self, sandbox_id: &str) {
        let expires = Instant::now() + Duration::from_secs(self.config.sandbox_timeout_secs);
        self.lease_expires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(sandbox_id.to_string(), expires);
    }

    fn unregister_sandbox_lease(&self, sandbox_id: &str) {
        self.lease_expires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(sandbox_id);
    }

    /// Background lease touch for idle sandboxes (no-op when TTL still > 5m).
    pub fn spawn_lease_ticker(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let ids: Vec<String> = self
                    .lease_expires
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .keys()
                    .cloned()
                    .collect();
                for sandbox_id in ids {
                    if let Err(e) = self.touch_sandbox_lease(&sandbox_id).await {
                        warn!(
                            target: "claw_fc_sandbox",
                            sandbox_id = %sandbox_id,
                            error = %e,
                            "lease ticker touch failed"
                        );
                    }
                }
            }
        });
    }

    #[must_use]
    pub fn config(&self) -> &FcSandboxConfig {
        &self.config
    }

    fn auth_headers(&self) -> Result<HeaderMap, String> {
        let mut headers = HeaderMap::new();
        if self.config.is_self_hosted() {
            headers.insert(
                "X-API-Key",
                HeaderValue::from_str(self.config.api_key.trim())
                    .map_err(|e| format!("X-API-Key header: {e}"))?,
            );
        } else {
            let value = format!("Bearer {}", self.config.api_key);
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&value).map_err(|e| format!("auth header: {e}"))?,
            );
        }
        Ok(headers)
    }

    fn ttyd_public_host(&self, sandbox_id: &str, sandbox_domain: &str) -> String {
        self.service_public_host(self.config.ttyd_port, sandbox_id, sandbox_domain)
    }

    /// Host for a published sandbox port (`{port}-{sandboxId}.{domain}`).
    #[must_use]
    pub fn service_public_host(&self, port: u16, sandbox_id: &str, sandbox_domain: &str) -> String {
        format!("{port}-{sandbox_id}.{sandbox_domain}")
    }

    /// Self-hosted on `WireGuard`: skip traffic token (see `docs/ovs-chat/E2B-TRAFFIC-ROUTING-F14.md`).
    fn apply_self_hosted_create_opts(&self, body: &mut Value) {
        if self.config.is_self_hosted() {
            body["secure"] = json!(false);
        }
    }

    /// Append `?token=` / `&token=` when the sandbox has a traffic access token.
    #[must_use]
    pub fn traffic_url(base: &str, token: Option<&str>) -> String {
        let Some(token) = token.map(str::trim).filter(|t| !t.is_empty()) else {
            return base.to_string();
        };
        if base.contains('?') {
            format!("{base}&token={token}")
        } else {
            format!("{base}?token={token}")
        }
    }

    /// HTTP base for OVS singleton (`http(s)://{port}-{sandboxId}.{domain}/ovs`).
    #[must_use]
    pub fn ovs_public_base_url(&self, handle: &FcSandboxHandle) -> String {
        let scheme = if self.config.is_self_hosted() {
            "http"
        } else {
            "https"
        };
        let host = self.service_public_host(
            self.config.ovs_port,
            &handle.sandbox_id,
            &handle.sandbox_domain,
        );
        format!("{scheme}://{host}/ovs")
    }

    /// Create a sandbox with session affinity metadata (`sessionId` key).
    pub async fn create_sandbox(
        &self,
        session_id: &str,
        proj_id: i64,
        ovs_mode: bool,
    ) -> Result<FcSandboxHandle, String> {
        let mut metadata = BTreeMap::new();
        metadata.insert("sessionId".to_string(), session_id.to_string());
        metadata.insert("projId".to_string(), proj_id.to_string());

        let mut body = json!({
            "templateID": self.config.template,
            "timeout": self.config.sandbox_timeout_secs,
            "metadata": metadata,
        });
        if let Some(nas) = self.nas_config_json(session_id, proj_id, ovs_mode) {
            body["nasConfig"] = nas;
        } else if let Some(mounts) = self.legacy_volume_mounts_json(ovs_mode) {
            body["volumeMounts"] = mounts;
        }
        self.apply_self_hosted_create_opts(&mut body);

        let url = format!("{}/sandboxes", self.config.api_url);
        debug!(target: "claw_fc_sandbox", %url, template = %self.config.template, "create sandbox");
        let resp = self
            .http
            .post(&url)
            .headers(self.auth_headers()?)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("fc create sandbox request: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("fc create sandbox body: {e}"))?;
        if !status.is_success() {
            return Err(format!("fc create sandbox HTTP {status}: {text}"));
        }

        let parsed: CreateSandboxResponse = serde_json::from_str(&text)
            .map_err(|e| format!("fc create sandbox parse: {e}; body={text}"))?;
        let sandbox_domain = if self.config.is_self_hosted() {
            self.config.domain.clone()
        } else {
            parsed
                .domain
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| self.config.domain.clone())
        };
        let ttyd_public_host = self.ttyd_public_host(&parsed.sandbox_id, &sandbox_domain);
        let handle = FcSandboxHandle {
            sandbox_id: parsed.sandbox_id,
            sandbox_domain,
            envd_access_token: parsed.envd_access_token,
            traffic_access_token: parsed.traffic_access_token,
            ttyd_public_host,
            ttyd_use_tls: !self.config.is_self_hosted(),
        };
        self.register_sandbox_lease(&handle.sandbox_id);
        Ok(handle)
    }

    /// Create the cluster OVS singleton sandbox (`metadata.clawRole=ovs-singleton`).
    pub async fn create_ovs_singleton_sandbox(
        &self,
        cluster_id: &str,
    ) -> Result<FcSandboxHandle, String> {
        let mut metadata = BTreeMap::new();
        metadata.insert("clawRole".to_string(), "ovs-singleton".to_string());
        metadata.insert("clusterId".to_string(), cluster_id.to_string());

        let mut body = json!({
            "templateID": self.config.ovs_template,
            "timeout": self.config.sandbox_timeout_secs,
            "metadata": metadata,
        });
        if let Some(nas) = self.nas_config_ovs_root_json() {
            body["nasConfig"] = nas;
        }
        self.apply_self_hosted_create_opts(&mut body);

        let url = format!("{}/sandboxes", self.config.api_url);
        debug!(target: "claw_fc_sandbox", %url, cluster_id, "create ovs singleton sandbox");
        let resp = self
            .http
            .post(&url)
            .headers(self.auth_headers()?)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("fc create ovs sandbox request: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("fc create ovs sandbox body: {e}"))?;
        if !status.is_success() {
            return Err(format!("fc create ovs sandbox HTTP {status}: {text}"));
        }

        let parsed: CreateSandboxResponse = serde_json::from_str(&text)
            .map_err(|e| format!("fc create ovs sandbox parse: {e}; body={text}"))?;
        let sandbox_domain = if self.config.is_self_hosted() {
            self.config.domain.clone()
        } else {
            parsed
                .domain
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| self.config.domain.clone())
        };
        let ttyd_public_host = self.ttyd_public_host(&parsed.sandbox_id, &sandbox_domain);
        let handle = FcSandboxHandle {
            sandbox_id: parsed.sandbox_id,
            sandbox_domain,
            envd_access_token: parsed.envd_access_token,
            traffic_access_token: parsed.traffic_access_token,
            ttyd_public_host,
            ttyd_use_tls: !self.config.is_self_hosted(),
        };
        self.register_sandbox_lease(&handle.sandbox_id);
        Ok(handle)
    }

    /// Create a project-bound warm worker (`metadata.clawRole=warm-proj`; NAS mounts `/claw_ds` only).
    pub async fn create_warm_proj_sandbox(&self, proj_id: i64) -> Result<FcSandboxHandle, String> {
        let warm_session_id = format!("warm-proj-{proj_id}");
        let mut metadata = BTreeMap::new();
        metadata.insert("projId".to_string(), proj_id.to_string());
        metadata.insert("sessionId".to_string(), warm_session_id);
        metadata.insert("clawRole".to_string(), "warm-proj".to_string());

        let mut body = json!({
            "templateID": self.config.template,
            "timeout": self.config.sandbox_timeout_secs,
            "metadata": metadata,
        });
        if let Some(nas) = self.nas_config_proj_json(proj_id) {
            body["nasConfig"] = nas;
        } else if let Some(mounts) = self.legacy_volume_mounts_proj_json() {
            body["volumeMounts"] = mounts;
        }
        self.apply_self_hosted_create_opts(&mut body);

        let url = format!("{}/sandboxes", self.config.api_url);
        debug!(target: "claw_fc_sandbox", %url, proj_id, "create warm-proj sandbox");
        let resp = self
            .http
            .post(&url)
            .headers(self.auth_headers()?)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("fc create warm sandbox request: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("fc create warm sandbox body: {e}"))?;
        if !status.is_success() {
            return Err(format!("fc create warm sandbox HTTP {status}: {text}"));
        }

        let parsed: CreateSandboxResponse = serde_json::from_str(&text)
            .map_err(|e| format!("fc create warm sandbox parse: {e}; body={text}"))?;
        let sandbox_domain = if self.config.is_self_hosted() {
            self.config.domain.clone()
        } else {
            parsed
                .domain
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| self.config.domain.clone())
        };
        let ttyd_public_host = self.ttyd_public_host(&parsed.sandbox_id, &sandbox_domain);
        let handle = FcSandboxHandle {
            sandbox_id: parsed.sandbox_id,
            sandbox_domain,
            envd_access_token: parsed.envd_access_token,
            traffic_access_token: parsed.traffic_access_token,
            ttyd_public_host,
            ttyd_use_tls: !self.config.is_self_hosted(),
        };
        self.register_sandbox_lease(&handle.sandbox_id);
        Ok(handle)
    }

    /// Reset sandbox TTL (`POST /sandboxes/{id}/timeout`).
    pub async fn set_sandbox_timeout(
        &self,
        sandbox_id: &str,
        timeout_secs: u64,
    ) -> Result<(), String> {
        let url = format!(
            "{}/sandboxes/{}/timeout",
            self.config.api_url.trim_end_matches('/'),
            sandbox_id
        );
        debug!(
            target: "claw_fc_sandbox",
            %url,
            sandbox_id,
            timeout_secs,
            "set sandbox timeout"
        );
        let resp = self
            .http
            .post(&url)
            .headers(self.auth_headers()?)
            .json(&json!({ "timeout": timeout_secs }))
            .send()
            .await
            .map_err(|e| format!("fc set sandbox timeout request: {e}"))?;
        if resp.status().is_success() || resp.status().as_u16() == 204 {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("fc set sandbox timeout HTTP {status}: {text}"))
    }

    /// Renew TTL when remaining time is under [`SANDBOX_LEASE_RENEW_LEAD_SECS`] (5 minutes).
    pub async fn touch_sandbox_lease(&self, sandbox_id: &str) -> Result<(), String> {
        let timeout_secs = self.config.sandbox_timeout_secs;
        let now = Instant::now();
        let should_renew = {
            let guard = self
                .lease_expires
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match guard.get(sandbox_id) {
                Some(expires_at) => {
                    Self::lease_should_renew(expires_at.saturating_duration_since(now).as_secs())
                }
                None => true,
            }
        };
        if !should_renew {
            return Ok(());
        }
        self.set_sandbox_timeout(sandbox_id, timeout_secs).await?;
        self.lease_expires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                sandbox_id.to_string(),
                now + Duration::from_secs(timeout_secs),
            );
        debug!(
            target: "claw_fc_sandbox",
            sandbox_id,
            timeout_secs,
            "sandbox lease renewed"
        );
        Ok(())
    }

    /// Kill a sandbox (`DELETE /sandboxes/{id}`).
    pub async fn kill_sandbox(&self, sandbox_id: &str) -> Result<(), String> {
        let url = format!("{}/sandboxes/{}", self.config.api_url, sandbox_id);
        let resp = self
            .http
            .delete(&url)
            .headers(self.auth_headers()?)
            .send()
            .await
            .map_err(|e| format!("fc kill sandbox request: {e}"))?;
        self.unregister_sandbox_lease(sandbox_id);
        if resp.status().is_success() || resp.status().as_u16() == 404 {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("fc kill sandbox HTTP {status}: {text}"))
    }

    /// Run `claw gateway-solve-once` inside an FC sandbox.
    pub async fn exec_gateway_solve_once(
        &self,
        sandbox_id: &str,
        task_rel_under_root: &str,
        claw_bin: &str,
        env: BTreeMap<String, String>,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<FcExecOutcome, String> {
        self.touch_sandbox_lease(sandbox_id).await?;
        let task_file = format!("/claw_host_root/{task_rel_under_root}");
        let payload = json!({
            "op": "exec_solve",
            "api_key": self.config.api_key,
            "domain": self.config.domain,
            "api_url": self.config.api_url,
            "sandbox_url": self.config.sandbox_url,
            "sandbox_id": sandbox_id,
            "claw_bin": claw_bin,
            "task_file": task_file,
            "env": env,
            "timeout": 600,
            "nas_tools_rel": self.config.nas_tools_rel,
            "self_hosted": self.config.is_self_hosted(),
        });
        Self::run_exec_helper(&self.config.exec_helper, &payload, on_stdout_line).await
    }

    /// Run a shell script inside the sandbox via `deploy/fc-sandbox/fc_exec.py` (envd gRPC).
    pub async fn exec_shell_script(
        &self,
        handle: &FcSandboxHandle,
        script: &str,
    ) -> Result<(), String> {
        self.touch_sandbox_lease(&handle.sandbox_id).await?;
        let payload = json!({
            "op": "run_sh",
            "api_key": self.config.api_key,
            "domain": handle.sandbox_domain,
            "api_url": self.config.api_url,
            "sandbox_url": self.config.sandbox_url,
            "sandbox_id": handle.sandbox_id,
            "script": script,
            "nas_tools_rel": self.config.nas_tools_rel,
            "self_hosted": self.config.is_self_hosted(),
        });
        Self::run_exec_helper(&self.config.exec_helper, &payload, None)
            .await
            .map(|_| ())
    }

    async fn run_exec_helper(
        helper: &Path,
        payload: &Value,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<FcExecOutcome, String> {
        if !helper.is_file() {
            return Err(format!(
                "fc exec helper not found at {} (set CLAW_FC_EXEC_HELPER)",
                helper.display()
            ));
        }

        let mut child = Command::new("python3")
            .arg(helper)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn fc exec helper: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            let bytes = serde_json::to_vec(payload).map_err(|e| format!("exec payload: {e}"))?;
            stdin
                .write_all(&bytes)
                .await
                .map_err(|e| format!("fc exec stdin: {e}"))?;
        }

        let stderr = child.stderr.take().expect("stderr piped");
        let stdout = child.stdout.take().expect("stdout piped");

        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            let mut acc = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => acc.push_str(&line),
                }
            }
            acc
        });

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let mut outcome: Option<FcExecOutcome> = None;
        let mut helper_error: Option<String> = None;

        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| format!("fc exec helper stdout read: {e}"))?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let parsed: Value = serde_json::from_str(trimmed)
                .map_err(|e| format!("fc exec helper ndjson decode: {e}: {trimmed}"))?;
            match ingest_fc_exec_helper_line(&parsed, trimmed, on_stdout_line.as_ref()) {
                FcExecHelperIngest::More => {}
                FcExecHelperIngest::Outcome(done) => {
                    outcome = Some(done);
                    break;
                }
                FcExecHelperIngest::Error(err) => {
                    helper_error = Some(err);
                    break;
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| format!("fc exec wait: {e}"))?;
        let stderr_acc = stderr_task.await.unwrap_or_default();

        if let Some(err) = helper_error {
            return Err(err);
        }
        if let Some(out) = outcome {
            if !status.success() && out.exit_code == 0 {
                warn!(
                    target: "claw_fc_sandbox",
                    stderr = %stderr_acc,
                    "fc exec helper exited non-zero but emitted ok outcome"
                );
            }
            return Ok(out);
        }
        if !status.success() {
            warn!(
                target: "claw_fc_sandbox",
                stderr = %stderr_acc,
                "fc exec helper failed without outcome envelope"
            );
            return Err(format!("fc exec helper exit {status}: {stderr_acc}"));
        }
        Err("fc exec helper: missing terminal outcome envelope".into())
    }

    /// Dynamic NAS at sandbox create — export root → `/claw_ws` (OVS singleton).
    fn nas_config_ovs_root_json(&self) -> Option<Value> {
        let server = self.config.nas_server.as_ref()?;
        let export = self.config.nas_export.as_deref().unwrap_or("/");
        Some(json!({
            "userId": self.config.nas_user_id,
            "groupId": self.config.nas_group_id,
            "mountPoints": [json!({
                "serverAddr": nas_server_addr(server, export, ""),
                "mountDir": "/claw_ws",
            })],
        }))
    }

    /// Dynamic NAS at sandbox create — project home only (`/claw_ds`) for warm pool.
    fn nas_config_proj_json(&self, proj_id: i64) -> Option<Value> {
        let server = self.config.nas_server.as_ref()?;
        let export = self.config.nas_export.as_deref().unwrap_or("/");
        let proj_home_rel = format!("proj_{proj_id}/home");
        Some(json!({
            "userId": self.config.nas_user_id,
            "groupId": self.config.nas_group_id,
            "mountPoints": [json!({
                "serverAddr": nas_server_addr(server, export, &proj_home_rel),
                "mountDir": "/claw_ds",
            })],
        }))
    }

    /// Dynamic NAS at sandbox create (`nasConfig`); host-side mount when e2b supports it.
    fn nas_config_json(&self, session_id: &str, proj_id: i64, ovs_mode: bool) -> Option<Value> {
        let server = self.config.nas_server.as_ref()?;
        let export = self.config.nas_export.as_deref().unwrap_or("/");
        let session_rel = format!("proj_{proj_id}/sessions/{session_id}");
        let mut mount_points = vec![json!({
            "serverAddr": nas_server_addr(server, export, &session_rel),
            "mountDir": "/claw_host_root",
        })];
        if ovs_mode {
            let proj_home_rel = format!("proj_{proj_id}/home");
            mount_points.push(json!({
                "serverAddr": nas_server_addr(server, export, &proj_home_rel),
                "mountDir": "/claw_ds",
            }));
        }
        Some(json!({
            "userId": self.config.nas_user_id,
            "groupId": self.config.nas_group_id,
            "mountPoints": mount_points,
        }))
    }

    /// Legacy volume — project home only for warm pool.
    fn legacy_volume_mounts_proj_json(&self) -> Option<Value> {
        let vol = self.config.nas_volume_name.as_ref()?;
        Some(json!([json!({ "name": vol, "path": "/claw_ds" })]))
    }

    /// Legacy: template pre-registered volume name (`volumeMounts`); needs custom template in console.
    fn legacy_volume_mounts_json(&self, ovs_mode: bool) -> Option<Value> {
        let vol = self.config.nas_volume_name.as_ref()?;
        let mounts: Vec<Value> = if ovs_mode {
            vec![
                json!({ "name": vol, "path": "/claw_ds" }),
                json!({ "name": vol, "path": "/claw_host_root" }),
            ]
        } else {
            vec![json!({ "name": vol, "path": "/claw_host_root" })]
        };
        Some(json!(mounts))
    }

    /// Build volume mounts for OVS session when `CLAW_FC_NAS_VOLUME_NAME` is set (legacy).
    #[must_use]
    pub fn default_volume_mounts(&self, ovs_mode: bool) -> Vec<FcSandboxVolumeMount> {
        let Some(ref vol) = self.config.nas_volume_name else {
            return Vec::new();
        };
        if ovs_mode {
            vec![
                FcSandboxVolumeMount {
                    name: vol.clone(),
                    path: "/claw_ds".into(),
                },
                FcSandboxVolumeMount {
                    name: vol.clone(),
                    path: "/claw_host_root".into(),
                },
            ]
        } else {
            vec![FcSandboxVolumeMount {
                name: vol.clone(),
                path: "/claw_host_root".into(),
            }]
        }
    }
}

/// NAS `serverAddr` for FC `nasConfig.mountPoints` (`host:export/rel`).
#[must_use]
pub fn nas_server_addr(nas_server: &str, export: &str, rel_path: &str) -> String {
    let host = nas_server.trim().trim_end_matches('/');
    let rel = rel_path.trim_start_matches('/');
    let export = export.trim();
    if export.is_empty() || export == "/" {
        format!("{host}:/{rel}")
    } else {
        let export = export.trim_start_matches('/').trim_end_matches('/');
        format!("{host}:/{export}/{rel}")
    }
}

#[cfg(test)]
mod nas_addr_tests {
    use super::nas_server_addr;

    #[test]
    fn nas_addr_root_export() {
        assert_eq!(
            nas_server_addr("a.cn-beijing.nas.aliyuncs.com", "/", "proj_1/home"),
            "a.cn-beijing.nas.aliyuncs.com:/proj_1/home"
        );
    }

    #[test]
    fn nas_addr_sub_export() {
        assert_eq!(
            nas_server_addr(
                "a.cn-beijing.nas.aliyuncs.com",
                "/claw-workspace",
                "proj_1/sessions/ovs-1"
            ),
            "a.cn-beijing.nas.aliyuncs.com:/claw-workspace/proj_1/sessions/ovs-1"
        );
    }
}

#[cfg(test)]
mod client_tests {
    use super::*;

    #[test]
    fn lease_should_renew_threshold() {
        assert!(!FcSandboxClient::lease_should_renew(300));
        assert!(FcSandboxClient::lease_should_renew(299));
        assert!(FcSandboxClient::lease_should_renew(0));
    }

    #[test]
    fn ttyd_host_format() {
        let cfg = FcSandboxConfig {
            api_key: "e2b_test".into(),
            api_url: "https://api.cn-beijing.e2b.fc.aliyuncs.com".into(),
            sandbox_url: None,
            domain: "cn-beijing.e2b.fc.aliyuncs.com".into(),
            template: "code-interpreter-v1".into(),
            sandbox_timeout_secs: 300,
            nas_server: None,
            nas_export: None,
            nas_volume_name: None,
            nas_tools_rel: ".claw-fc-tools".into(),
            nas_user_id: 1000,
            nas_group_id: 1000,
            exec_helper: "deploy/fc-sandbox/fc_exec.py".into(),
            ttyd_port: 7681,
            ovs_template: "claw-ovs".into(),
            ovs_port: 3000,
        };
        let c = FcSandboxClient::new(cfg);
        assert_eq!(
            c.ttyd_public_host("sbx-abc", "cn-beijing.e2b.fc.aliyuncs.com"),
            "7681-sbx-abc.cn-beijing.e2b.fc.aliyuncs.com"
        );
    }
}
