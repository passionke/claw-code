//! E2B-compatible REST client for Alibaba e2b cloud sandbox. Author: kejiqing

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

/// Background TTL touch interval for tracked project workers (`spawn_lease_ticker`).
pub const SANDBOX_LEASE_TICK_SECS: u64 = 60;

/// POST /timeout then GET /sandboxes/{id} retries when `endAt` did not expand.
const SANDBOX_TTL_VERIFY_MAX_ATTEMPTS: u32 = 3;

/// Platform snapshot from `GET /sandboxes/{id}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxSnapshot {
    pub state: String,
    pub end_at_ms: Option<i64>,
}

impl SandboxSnapshot {
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state == "running"
    }

    #[must_use]
    pub fn remaining_ttl_secs(&self, now_ms: i64) -> Option<u64> {
        let end = self.end_at_ms?;
        if end <= now_ms {
            return Some(0);
        }
        Some(u64::try_from((end - now_ms) / 1000).unwrap_or(0))
    }
}

use crate::config::E2bSandboxConfig;
use crate::e2b_platform::{fetch_e2b_platform_nas, E2bNasPlatform};
use crate::nas_paths::{self, warm_worker_mounts, worker_mounts, NasMountPoint};
use crate::types::{CreateSandboxResponse, E2bExecOutcome, E2bSandboxHandle, GatewaySolveInputs};

enum FcExecHelperIngest {
    More,
    Outcome(E2bExecOutcome),
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
            return FcExecHelperIngest::Outcome(E2bExecOutcome {
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
        return FcExecHelperIngest::Outcome(E2bExecOutcome {
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

/// Shell probe: each `nasConfig.mountDir` must answer `mountpoint -q`.
#[must_use]
fn guest_nas_mount_probe_script(mount_dirs: &[&str]) -> String {
    let mut lines = vec!["set -eu".to_string(), "missing=''".to_string()];
    for dir in mount_dirs {
        lines.push(format!(
            r#"if ! mountpoint -q "{dir}" 2>/dev/null; then
  echo "e2b nas bind missing: {dir} is not a mountpoint" >&2
  ls -ld "{dir}" 2>/dev/null >&2 || echo "  (path does not exist)" >&2
  missing="$missing {dir}"
fi"#
        ));
    }
    lines.push(
        r#"if [ -n "$missing" ]; then
  echo "e2b nas bind probe: e2b ignored nasConfig at POST /sandboxes (guest mountDirs missing). Fix e2bserver: hostMountRoot+relPath direct bind; Gateway mkdir on NAS first. Run deploy/stack/lib/verify-e2b-nas-inject.sh" >&2
  exit 1
fi"#
            .to_string(),
    );
    lines.join("\n")
}

/// HTTP client for e2b sandbox lifecycle + delegated envd exec.
#[derive(Debug, Clone)]
pub struct E2bSandboxClient {
    config: E2bSandboxConfig,
    http: reqwest::Client,
    /// Local TTL estimate per sandbox (`POST /timeout` resets from request time).
    lease_expires: Arc<Mutex<HashMap<String, Instant>>>,
    /// e2b `GET /health` NAS platform (host bind inject).
    e2b_platform_nas: Arc<Mutex<Option<E2bNasPlatform>>>,
}

impl E2bSandboxClient {
    #[must_use]
    pub fn new(config: E2bSandboxConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
            lease_expires: Arc::new(Mutex::new(HashMap::new())),
            e2b_platform_nas: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn config(&self) -> &E2bSandboxConfig {
        &self.config
    }

    /// Refresh self-hosted e2b platform NAS from `GET /health` (pool uses before create).
    pub async fn refresh_e2b_platform_nas(&self) -> Result<(), String> {
        if !self.config.is_self_hosted() {
            return Ok(());
        }
        match fetch_e2b_platform_nas(&self.http, &self.config.api_url, &self.config.api_key).await {
            Ok(Some(platform)) => {
                let ready = platform.ready;
                let mount_source = platform.mount_source.clone();
                *self
                    .e2b_platform_nas
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(platform);
                if ready {
                    tracing::info!(
                        target: "claw_e2b_sandbox",
                        %mount_source,
                        "e2b platform NAS ready (nasConfig bind inject)"
                    );
                } else {
                    tracing::warn!(
                        target: "claw_e2b_sandbox",
                        %mount_source,
                        "e2b platform NAS not ready (health nas.ready=false)"
                    );
                }
                Ok(())
            }
            Ok(None) => Err(
                "e2b GET /health missing nas block (hostMountRoot + sandboxInject=bind required)"
                    .into(),
            ),
            Err(e) => Err(e),
        }
    }

    /// True when e2b host injects NAS into sandbox via bind (`nas.ready` + hostMountRoot).
    #[must_use]
    pub fn e2b_nas_injects_vm(&self) -> bool {
        self.e2b_platform_nas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|p| p.ready && p.uses_host_bind_inject())
    }

    async fn prepare_self_hosted_create(&self) -> Result<(), String> {
        if !self.config.is_self_hosted() {
            return Ok(());
        }
        if let Err(e) = self.refresh_e2b_platform_nas().await {
            return Err(format!("e2b platform health: {e}"));
        }
        if !self.e2b_nas_injects_vm() {
            return Err(
                "e2b NAS bind not ready (GET /health: nas.ready + hostMountRoot + sandboxInject=bind)"
                    .into(),
            );
        }
        Ok(())
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

    fn now_ms() -> i64 {
        chrono::Utc::now().timestamp_millis()
    }

    #[must_use]
    pub fn min_verified_remaining_secs(requested_timeout_secs: u64) -> u64 {
        requested_timeout_secs.saturating_sub(SANDBOX_LEASE_RENEW_LEAD_SECS.saturating_mul(2))
    }

    fn parse_end_at_ms(body: &Value) -> Option<i64> {
        let raw = body
            .get("endAt")
            .or_else(|| body.get("end_at"))
            .and_then(Value::as_str)?;
        chrono::DateTime::parse_from_rfc3339(raw)
            .ok()
            .map(|dt| dt.timestamp_millis())
    }

    fn sync_local_lease_from_end_at(&self, sandbox_id: &str, end_at_ms: i64) {
        let now_ms = Self::now_ms();
        let remaining_secs = if end_at_ms <= now_ms {
            0
        } else {
            u64::try_from((end_at_ms - now_ms) / 1000).unwrap_or(0)
        };
        self.lease_expires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                sandbox_id.to_string(),
                Instant::now() + Duration::from_secs(remaining_secs),
            );
    }

    /// Track a sandbox for [`Self::spawn_lease_ticker`] (project workers, warm sandboxes).
    pub fn register_tracked_sandbox(&self, sandbox_id: &str) {
        if sandbox_id.trim().is_empty() {
            return;
        }
        self.lease_expires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(sandbox_id.to_string())
            .or_insert_with(Instant::now);
    }

    pub fn register_tracked_sandboxes(&self, sandbox_ids: &[String]) {
        for id in sandbox_ids {
            self.register_tracked_sandbox(id);
        }
    }

    /// `GET /sandboxes/{id}` — state + `endAt` for TTL verification.
    pub async fn fetch_sandbox_snapshot(
        &self,
        sandbox_id: &str,
    ) -> Result<SandboxSnapshot, String> {
        let url = format!("{}/sandboxes/{}", self.config.api_url, sandbox_id);
        let resp = self
            .http
            .get(&url)
            .headers(self.auth_headers().unwrap_or_default())
            .send()
            .await
            .map_err(|e| format!("e2b get sandbox request: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "fc get sandbox HTTP {} for {sandbox_id}",
                resp.status()
            ));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("e2b get sandbox parse: {e}"))?;
        Ok(SandboxSnapshot {
            state: body
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            end_at_ms: Self::parse_end_at_ms(&body),
        })
    }

    /// POST /timeout then read `endAt`; retry when platform did not expand TTL.
    pub async fn renew_sandbox_ttl_verified(
        &self,
        sandbox_id: &str,
        timeout_secs: u64,
    ) -> Result<SandboxSnapshot, String> {
        let min_remaining = Self::min_verified_remaining_secs(timeout_secs);
        let mut last_snap: Option<SandboxSnapshot> = None;
        for attempt in 1..=SANDBOX_TTL_VERIFY_MAX_ATTEMPTS {
            self.set_sandbox_timeout(sandbox_id, timeout_secs).await?;
            if attempt < SANDBOX_TTL_VERIFY_MAX_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
            }
            let snap = self.fetch_sandbox_snapshot(sandbox_id).await?;
            last_snap = Some(snap.clone());
            if !snap.is_running() {
                return Err(format!(
                    "sandbox {sandbox_id} not running after set_timeout (state={})",
                    snap.state
                ));
            }
            let now_ms = Self::now_ms();
            let remaining = snap.remaining_ttl_secs(now_ms).unwrap_or(0);
            if remaining >= min_remaining {
                if let Some(end_at_ms) = snap.end_at_ms {
                    self.sync_local_lease_from_end_at(sandbox_id, end_at_ms);
                } else {
                    let now = Instant::now();
                    self.lease_expires
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(
                            sandbox_id.to_string(),
                            now + Duration::from_secs(timeout_secs),
                        );
                }
                self.register_tracked_sandbox(sandbox_id);
                debug!(
                    target: "claw_e2b_sandbox",
                    sandbox_id,
                    timeout_secs,
                    remaining_secs = remaining,
                    end_at_ms = ?snap.end_at_ms,
                    attempt,
                    "sandbox TTL renewed and verified via endAt"
                );
                return Ok(snap);
            }
            warn!(
                target: "claw_e2b_sandbox",
                sandbox_id,
                timeout_secs,
                remaining_secs = remaining,
                min_remaining_secs = min_remaining,
                attempt,
                end_at_ms = ?snap.end_at_ms,
                "sandbox endAt below expected after set_timeout; retrying"
            );
        }
        Err(format!(
            "sandbox {sandbox_id} TTL verify failed after {SANDBOX_TTL_VERIFY_MAX_ATTEMPTS} attempts (last endAt={:?})",
            last_snap.and_then(|s| s.end_at_ms)
        ))
    }

    /// Background lease touch for tracked sandboxes (60s tick; verifies `endAt` on self-hosted).
    pub fn spawn_lease_ticker(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(SANDBOX_LEASE_TICK_SECS));
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
                            target: "claw_e2b_sandbox",
                            sandbox_id = %sandbox_id,
                            error = %e,
                            "lease ticker touch failed"
                        );
                    }
                }
            }
        });
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

    /// After `POST /sandboxes` + `nasConfig`: every `mountDir` must be a mountpoint in the guest.
    async fn finish_sandbox_create(
        &self,
        handle: E2bSandboxHandle,
        nas_configured: bool,
        mount_points: &[NasMountPoint],
    ) -> Result<E2bSandboxHandle, String> {
        if nas_configured {
            if let Err(e) = self.assert_guest_nas_mounts(&handle, mount_points).await {
                let _ = self.kill_sandbox(&handle.sandbox_id).await;
                return Err(e);
            }
        }
        Ok(handle)
    }

    async fn assert_guest_nas_mounts(
        &self,
        handle: &E2bSandboxHandle,
        mount_points: &[NasMountPoint],
    ) -> Result<(), String> {
        let dirs: Vec<&str> = mount_points.iter().map(|m| m.mount_dir.as_str()).collect();
        if dirs.is_empty() {
            return Ok(());
        }
        let script = guest_nas_mount_probe_script(&dirs);
        self.exec_shell_script(handle, &script).await.map_err(|e| {
            format!(
                "sandbox {} nasConfig bind not mounted in guest ({}): {e}",
                handle.sandbox_id,
                dirs.join(", ")
            )
        })
    }

    /// Create a sandbox with session affinity metadata (`sessionId` key).
    pub async fn create_sandbox(
        &self,
        cluster_id: &str,
        session_id: &str,
        session_segment: &str,
        proj_id: i64,
        ovs_mode: bool,
        worker_id: &str,
    ) -> Result<E2bSandboxHandle, String> {
        self.prepare_self_hosted_create().await?;
        let mut metadata = BTreeMap::new();
        metadata.insert("sessionId".to_string(), session_id.to_string());
        metadata.insert("sessionSegment".to_string(), session_segment.to_string());
        metadata.insert("workerId".to_string(), worker_id.to_string());
        metadata.insert("projId".to_string(), proj_id.to_string());

        let mount_points = worker_mounts(cluster_id, proj_id, worker_id, ovs_mode);
        let mut body = json!({
            "templateID": self.config.template,
            "timeout": self.config.sandbox_timeout_secs,
            "metadata": metadata,
        });
        let nas = self.require_nas_config_body(&mount_points)?;
        body["nasConfig"] = json!(nas);
        let nas_configured = true;
        self.apply_self_hosted_create_opts(&mut body);

        let url = format!("{}/sandboxes", self.config.api_url);
        debug!(target: "claw_e2b_sandbox", %url, template = %self.config.template, "create sandbox");
        let resp = self
            .http
            .post(&url)
            .headers(self.auth_headers()?)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("e2b create sandbox request: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("e2b create sandbox body: {e}"))?;
        if !status.is_success() {
            return Err(format!("e2b create sandbox HTTP {status}: {text}"));
        }

        let parsed: CreateSandboxResponse = serde_json::from_str(&text)
            .map_err(|e| format!("e2b create sandbox parse: {e}; body={text}"))?;
        let sandbox_domain = if self.config.is_self_hosted() {
            self.config.domain.clone()
        } else {
            parsed
                .domain
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| self.config.domain.clone())
        };
        let ttyd_public_host = self.ttyd_public_host(&parsed.sandbox_id, &sandbox_domain);
        let handle = E2bSandboxHandle {
            sandbox_id: parsed.sandbox_id,
            sandbox_domain,
            envd_access_token: parsed.envd_access_token,
            traffic_access_token: parsed.traffic_access_token,
            ttyd_public_host,
            ttyd_use_tls: !self.config.is_self_hosted(),
        };
        self.register_sandbox_lease(&handle.sandbox_id);
        self.finish_sandbox_create(handle, nas_configured, &mount_points)
            .await
    }

    /// Create a project-bound warm worker (`metadata.clawRole=warm-proj`).
    pub async fn create_warm_proj_sandbox(
        &self,
        cluster_id: &str,
        proj_id: i64,
        worker_id: &str,
    ) -> Result<E2bSandboxHandle, String> {
        self.prepare_self_hosted_create().await?;
        let warm_session_id = format!("warm-proj-{proj_id}");
        let mut metadata = BTreeMap::new();
        metadata.insert("projId".to_string(), proj_id.to_string());
        metadata.insert("workerId".to_string(), worker_id.to_string());
        metadata.insert("sessionId".to_string(), warm_session_id);
        metadata.insert("clawRole".to_string(), "warm-proj".to_string());

        let mount_points = warm_worker_mounts(cluster_id, proj_id, worker_id);
        let mut body = json!({
            "templateID": self.config.template,
            "timeout": self.config.sandbox_timeout_secs,
            "metadata": metadata,
        });
        let nas = self.require_nas_config_body(&mount_points)?;
        body["nasConfig"] = json!(nas);
        let nas_configured = true;
        self.apply_self_hosted_create_opts(&mut body);

        let url = format!("{}/sandboxes", self.config.api_url);
        debug!(target: "claw_e2b_sandbox", %url, proj_id, "create warm-proj sandbox");
        let resp = self
            .http
            .post(&url)
            .headers(self.auth_headers()?)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("e2b create warm sandbox request: {e}"))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| format!("e2b create warm sandbox body: {e}"))?;
        if !status.is_success() {
            return Err(format!("e2b create warm sandbox HTTP {status}: {text}"));
        }

        let parsed: CreateSandboxResponse = serde_json::from_str(&text)
            .map_err(|e| format!("e2b create warm sandbox parse: {e}; body={text}"))?;
        let sandbox_domain = if self.config.is_self_hosted() {
            self.config.domain.clone()
        } else {
            parsed
                .domain
                .filter(|d| !d.trim().is_empty())
                .unwrap_or_else(|| self.config.domain.clone())
        };
        let ttyd_public_host = self.ttyd_public_host(&parsed.sandbox_id, &sandbox_domain);
        let handle = E2bSandboxHandle {
            sandbox_id: parsed.sandbox_id,
            sandbox_domain,
            envd_access_token: parsed.envd_access_token,
            traffic_access_token: parsed.traffic_access_token,
            ttyd_public_host,
            ttyd_use_tls: !self.config.is_self_hosted(),
        };
        self.register_sandbox_lease(&handle.sandbox_id);
        self.finish_sandbox_create(handle, nas_configured, &mount_points)
            .await
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
            target: "claw_e2b_sandbox",
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
            .map_err(|e| format!("e2b set sandbox timeout request: {e}"))?;
        if resp.status().is_success() || resp.status().as_u16() == 204 {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("e2b set sandbox timeout HTTP {status}: {text}"))
    }

    /// Renew TTL when remaining time is under [`SANDBOX_LEASE_RENEW_LEAD_SECS`] (5 minutes).
    pub async fn touch_sandbox_lease(&self, sandbox_id: &str) -> Result<(), String> {
        let timeout_secs = self.config.sandbox_timeout_secs;
        let now = Instant::now();
        let should_renew = if self.config.is_self_hosted() {
            true
        } else {
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
        self.renew_sandbox_ttl_verified(sandbox_id, timeout_secs)
            .await
            .map(|_| ())
    }

    /// True when e2b still has this sandbox running (`GET /sandboxes/{id}` → `state:"running"`).
    pub async fn sandbox_running(&self, sandbox_id: &str) -> bool {
        match self.fetch_sandbox_snapshot(sandbox_id).await {
            Ok(snap) => snap.is_running(),
            Err(_) => false,
        }
    }

    /// Renew sandbox TTL; verifies platform `endAt` after `POST /timeout`.
    pub async fn renew_sandbox_ttl_secs(
        &self,
        sandbox_id: &str,
        timeout_secs: u64,
    ) -> Result<(), String> {
        self.renew_sandbox_ttl_verified(sandbox_id, timeout_secs)
            .await
            .map(|_| ())
    }

    #[must_use]
    pub fn handle_to_json(handle: &E2bSandboxHandle) -> serde_json::Value {
        json!({
            "sandboxId": handle.sandbox_id,
            "sandboxDomain": handle.sandbox_domain,
            "envdAccessToken": handle.envd_access_token,
            "trafficAccessToken": handle.traffic_access_token,
            "ttydPublicHost": handle.ttyd_public_host,
            "ttydUseTls": handle.ttyd_use_tls,
        })
    }

    pub fn handle_from_json(value: &serde_json::Value) -> Result<E2bSandboxHandle, String> {
        let sandbox_id = value
            .get("sandboxId")
            .or_else(|| value.get("sandbox_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "handle_json missing sandboxId".to_string())?
            .to_string();
        let sandbox_domain = value
            .get("sandboxDomain")
            .or_else(|| value.get("sandbox_domain"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let envd_access_token = value
            .get("envdAccessToken")
            .or_else(|| value.get("envd_access_token"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let traffic_access_token = value
            .get("trafficAccessToken")
            .or_else(|| value.get("traffic_access_token"))
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let ttyd_public_host = value
            .get("ttydPublicHost")
            .or_else(|| value.get("ttyd_public_host"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let ttyd_use_tls = value
            .get("ttydUseTls")
            .or_else(|| value.get("ttyd_use_tls"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(!sandbox_domain.contains("10.8.0."));
        Ok(E2bSandboxHandle {
            sandbox_id,
            sandbox_domain,
            envd_access_token,
            traffic_access_token,
            ttyd_public_host,
            ttyd_use_tls,
        })
    }

    /// Gateway shutdown: DELETE leased sandboxes except persisted project workers.
    pub async fn kill_all_leased_sandboxes_except(&self, skip_sandbox_ids: &[String]) -> usize {
        let skip: std::collections::HashSet<&str> =
            skip_sandbox_ids.iter().map(String::as_str).collect();
        let ids: Vec<String> = self
            .lease_expires
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .filter(|id| !skip.contains(id.as_str()))
            .cloned()
            .collect();
        let mut killed = 0usize;
        for sid in ids {
            match self.kill_sandbox(&sid).await {
                Ok(()) => killed += 1,
                Err(e) => warn!(
                    target: "claw_e2b_sandbox",
                    sandbox_id = %sid,
                    error = %e,
                    "shutdown kill leased sandbox failed"
                ),
            }
        }
        killed
    }

    /// Gateway shutdown: DELETE every sandbox still in the lease registry (legacy).
    pub async fn kill_all_leased_sandboxes(&self) -> usize {
        self.kill_all_leased_sandboxes_except(&[]).await
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
            .map_err(|e| format!("e2b kill sandbox request: {e}"))?;
        self.unregister_sandbox_lease(sandbox_id);
        if resp.status().is_success() || resp.status().as_u16() == 404 {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("e2b kill sandbox HTTP {status}: {text}"))
    }

    /// Gateway shutdown: persistent singletons (observe/ovs) are Python-managed — no-op here.
    /// Gateway shutdown: persistent singletons (observe/ovs) are Python-managed — no-op here.
    #[allow(clippy::unused_async)]
    pub async fn kill_cluster_singleton_orphans(&self, _cluster_id: &str) -> Result<usize, String> {
        Ok(0)
    }

    /// Run `claw gateway-solve-once` inside an e2b sandbox.
    pub async fn exec_gateway_solve_once(
        &self,
        sandbox_id: &str,
        _task_rel_under_root: &str,
        claw_bin: &str,
        env: BTreeMap<String, String>,
        inputs: GatewaySolveInputs<'_>,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<E2bExecOutcome, String> {
        self.touch_sandbox_lease(sandbox_id).await?;
        let session_root = if inputs.session_segment.is_empty() {
            nas_paths::GUEST_CLAW_HOST_ROOT.to_string()
        } else {
            nas_paths::guest_session_root(inputs.session_segment)
        };
        let task_file = format!("{session_root}/gateway-solve-task.json");
        // Per-turn inputs travel inline; the worker lands them on its session mount. Author: kejiqing
        let payload = json!({
            "op": "exec_solve",
            "api_key": self.config.api_key,
            "domain": self.config.domain,
            "api_url": self.config.api_url,
            "sandbox_url": self.config.sandbox_url,
            "sandbox_id": sandbox_id,
            "claw_bin": claw_bin,
            "task_file": task_file,
            "task_json": inputs.task_json,
            "session_jsonl": inputs.session_jsonl,
            "session_segment": inputs.session_segment,
            "session_root": session_root,
            "env": env,
            "timeout": 600,
        });
        Self::run_exec_helper(&self.config.exec_helper, &payload, on_stdout_line).await
    }

    /// Run a shell script inside the sandbox via `deploy/e2b/e2b_exec.py` (envd gRPC).
    pub async fn exec_shell_script(
        &self,
        handle: &E2bSandboxHandle,
        script: &str,
    ) -> Result<(), String> {
        self.exec_shell_script_stdout(handle, script)
            .await
            .map(|_| ())
    }

    /// Like [`Self::exec_shell_script`] but streams stdout lines via NDJSON `stdout_line` events.
    pub async fn exec_shell_script_streaming(
        &self,
        handle: &E2bSandboxHandle,
        script: &str,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<E2bExecOutcome, String> {
        self.touch_sandbox_lease(&handle.sandbox_id).await?;
        let payload = json!({
            "op": "run_sh",
            "api_key": self.config.api_key,
            "domain": handle.sandbox_domain,
            "api_url": self.config.api_url,
            "sandbox_url": self.config.sandbox_url,
            "sandbox_id": handle.sandbox_id,
            "script": script,
        });
        Self::run_exec_helper(&self.config.exec_helper, &payload, on_stdout_line).await
    }

    /// Like [`Self::exec_shell_script`] but returns captured stdout (for small in-guest reads).
    pub async fn exec_shell_script_stdout(
        &self,
        handle: &E2bSandboxHandle,
        script: &str,
    ) -> Result<String, String> {
        self.touch_sandbox_lease(&handle.sandbox_id).await?;
        let payload = json!({
            "op": "run_sh",
            "api_key": self.config.api_key,
            "domain": handle.sandbox_domain,
            "api_url": self.config.api_url,
            "sandbox_url": self.config.sandbox_url,
            "sandbox_id": handle.sandbox_id,
            "script": script,
        });
        let outcome = Self::run_exec_helper(&self.config.exec_helper, &payload, None).await?;
        Ok(outcome.stdout)
    }

    #[allow(clippy::too_many_lines)]
    async fn run_exec_helper(
        helper: &Path,
        payload: &Value,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<E2bExecOutcome, String> {
        if !helper.is_file() {
            return Err(format!(
                "e2b exec helper not found at {} (set CLAW_E2B_EXEC_HELPER)",
                helper.display()
            ));
        }

        let mut child = Command::new("python3")
            .arg(helper)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn e2b exec helper: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            let bytes = serde_json::to_vec(payload).map_err(|e| format!("exec payload: {e}"))?;
            stdin
                .write_all(&bytes)
                .await
                .map_err(|e| format!("e2b exec stdin: {e}"))?;
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
        let mut outcome: Option<E2bExecOutcome> = None;
        let mut helper_error: Option<String> = None;
        let mut stdout_line_events = 0u32;

        loop {
            line.clear();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| format!("e2b exec helper stdout read: {e}"))?;
            if n == 0 {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let parsed: Value = serde_json::from_str(trimmed)
                .map_err(|e| format!("e2b exec helper ndjson decode: {e}: {trimmed}"))?;
            if parsed.get("ev").and_then(Value::as_str) == Some("stdout_line") {
                stdout_line_events = stdout_line_events.saturating_add(1);
            }
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
            .map_err(|e| format!("e2b exec wait: {e}"))?;
        let stderr_acc = stderr_task.await.unwrap_or_default();

        if let Some(err) = helper_error {
            return Err(err);
        }
        if let Some(out) = outcome {
            if stdout_line_events == 0 {
                if let Some(hook) = on_stdout_line.as_ref() {
                    if !out.stdout.is_empty() {
                        hook(out.stdout.clone());
                    }
                }
            }
            if !status.success() && out.exit_code == 0 {
                warn!(
                    target: "claw_e2b_sandbox",
                    stderr = %stderr_acc,
                    "fc exec helper exited non-zero but emitted ok outcome"
                );
            }
            return Ok(out);
        }
        if !status.success() {
            warn!(
                target: "claw_e2b_sandbox",
                stderr = %stderr_acc,
                "fc exec helper failed without outcome envelope"
            );
            return Err(format!("e2b exec helper exit {status}: {stderr_acc}"));
        }
        Err("fc exec helper: missing terminal outcome envelope".into())
    }

    fn nas_config_body(&self, mount_points: &[NasMountPoint]) -> Option<Value> {
        if mount_points.is_empty() {
            return None;
        }
        let platform = self
            .e2b_platform_nas
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let p = platform.filter(|p| p.ready && p.uses_host_bind_inject())?;
        let points: Vec<Value> = mount_points
            .iter()
            .map(|m| {
                let mut pt = json!({ "mountDir": m.mount_dir });
                if m.rel_path.is_empty() {
                    pt["relPath"] = json!("");
                } else {
                    pt["relPath"] = json!(m.rel_path);
                }
                if m.read_only {
                    pt["readOnly"] = json!(true);
                }
                pt
            })
            .collect();
        let mut body = json!({
            "userId": self.config.nas_user_id,
            "groupId": self.config.nas_group_id,
            "mountPoints": points,
        });
        if let Some(root) = Self::e2b_bind_host_mount_root(&p) {
            body["hostMountRoot"] = json!(root);
        } else {
            return None;
        }
        Some(body)
    }

    /// e2b **host** NAS mount (bind source), not Gateway `CLAW_NAS_HOST_MOUNT`.
    fn e2b_bind_host_mount_root(platform: &E2bNasPlatform) -> Option<String> {
        if let Ok(v) = std::env::var("CLAW_E2B_NAS_HOST_MOUNT") {
            let t = v.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
        platform.host_mount_root.clone()
    }

    fn require_nas_config_body(&self, mount_points: &[NasMountPoint]) -> Result<Value, String> {
        self.nas_config_body(mount_points).ok_or_else(|| {
            "e2b NAS bind not ready: GET /health must report nas.ready, hostMountRoot, sandboxInject=bind"
                .into()
        })
    }
}

/// NAS `serverAddr` for e2b `nasConfig.mountPoints` (`host:export/rel`).
#[cfg(test)]
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
    fn guest_nas_mount_probe_lists_mount_dirs() {
        let sh = guest_nas_mount_probe_script(&["/claw_ds", "/claw_host_root"]);
        assert!(sh.contains("mountpoint -q \"/claw_ds\""));
        assert!(sh.contains("mountpoint -q \"/claw_host_root\""));
        assert!(sh.contains("exit 1"));
    }

    #[test]
    fn lease_should_renew_threshold() {
        assert!(!E2bSandboxClient::lease_should_renew(300));
        assert!(E2bSandboxClient::lease_should_renew(299));
        assert!(E2bSandboxClient::lease_should_renew(0));
    }

    #[test]
    fn min_verified_remaining_secs_uses_renew_lead() {
        assert_eq!(
            E2bSandboxClient::min_verified_remaining_secs(3600),
            3600 - SANDBOX_LEASE_RENEW_LEAD_SECS * 2
        );
    }

    #[test]
    fn parse_end_at_ms_from_rfc3339() {
        let body = json!({ "endAt": "2027-06-29T09:25:09.303247652Z" });
        let ms = E2bSandboxClient::parse_end_at_ms(&body).expect("parse");
        assert!(ms > 1_700_000_000_000);
    }

    #[test]
    fn sandbox_snapshot_remaining_ttl() {
        let now = 1_000_000_000_000_i64;
        let snap = SandboxSnapshot {
            state: "running".into(),
            end_at_ms: Some(now + 90_000),
        };
        assert_eq!(snap.remaining_ttl_secs(now), Some(90));
    }

    #[test]
    fn ttyd_host_format() {
        let cfg = E2bSandboxConfig {
            api_key: "e2b_test".into(),
            api_url: "https://api.cn-beijing.e2b.fc.aliyuncs.com".into(),
            sandbox_url: None,
            domain: "cn-beijing.e2b.fc.aliyuncs.com".into(),
            template: "code-interpreter-v1".into(),
            sandbox_timeout_secs: 300,
            nas_server: None,
            nas_export: None,
            nas_user_id: 1000,
            nas_group_id: 1000,
            exec_helper: "deploy/e2b/e2b_exec.py".into(),
            ttyd_port: 7681,
            ovs_template: "claw-ovs".into(),
            ovs_port: 3000,
        };
        let c = E2bSandboxClient::new(cfg);
        assert_eq!(
            c.ttyd_public_host("sbx-abc", "cn-beijing.e2b.fc.aliyuncs.com"),
            "7681-sbx-abc.cn-beijing.e2b.fc.aliyuncs.com"
        );
    }
}
