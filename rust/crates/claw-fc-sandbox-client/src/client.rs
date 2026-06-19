//! E2B-compatible REST client for Alibaba FC cloud sandbox. Author: kejiqing

use std::collections::BTreeMap;
use std::path::Path;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};

use crate::config::FcSandboxConfig;
use crate::types::{CreateSandboxResponse, FcSandboxHandle, FcSandboxVolumeMount};

/// HTTP client for FC sandbox lifecycle + delegated envd exec.
#[derive(Debug, Clone)]
pub struct FcSandboxClient {
    config: FcSandboxConfig,
    http: reqwest::Client,
}

impl FcSandboxClient {
    #[must_use]
    pub fn new(config: FcSandboxConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    #[must_use]
    pub fn config(&self) -> &FcSandboxConfig {
        &self.config
    }

    fn auth_headers(&self) -> Result<HeaderMap, String> {
        let mut headers = HeaderMap::new();
        let value = format!("Bearer {}", self.config.api_key);
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&value).map_err(|e| format!("auth header: {e}"))?,
        );
        Ok(headers)
    }

    fn ttyd_public_host(&self, sandbox_id: &str, sandbox_domain: &str) -> String {
        format!(
            "{}-{}.{}",
            self.config.ttyd_port, sandbox_id, sandbox_domain
        )
    }

    /// Create a sandbox with session affinity metadata (`sessionId` key).
    pub async fn create_sandbox(
        &self,
        session_id: &str,
        proj_id: i64,
        volume_mounts: &[FcSandboxVolumeMount],
    ) -> Result<FcSandboxHandle, String> {
        let mut metadata = BTreeMap::new();
        metadata.insert("sessionId".to_string(), session_id.to_string());
        metadata.insert("projId".to_string(), proj_id.to_string());

        let mut body = json!({
            "templateID": self.config.template,
            "timeout": self.config.sandbox_timeout_secs,
            "metadata": metadata,
        });
        if !volume_mounts.is_empty() {
            let mounts: Vec<Value> = volume_mounts
                .iter()
                .map(|m| json!({ "name": m.name, "path": m.path }))
                .collect();
            body["volumeMounts"] = json!(mounts);
        }

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
        let sandbox_domain = parsed
            .domain
            .filter(|d| !d.trim().is_empty())
            .unwrap_or_else(|| self.config.domain.clone());
        let ttyd_public_host = self.ttyd_public_host(&parsed.sandbox_id, &sandbox_domain);
        Ok(FcSandboxHandle {
            sandbox_id: parsed.sandbox_id,
            sandbox_domain,
            envd_access_token: parsed.envd_access_token,
            traffic_access_token: parsed.traffic_access_token,
            ttyd_public_host,
            ttyd_use_tls: true,
        })
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
        if resp.status().is_success() || resp.status().as_u16() == 404 {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("fc kill sandbox HTTP {status}: {text}"))
    }

    /// Run a shell script inside the sandbox via `deploy/fc-sandbox/fc_exec.py` (envd gRPC).
    pub async fn exec_shell_script(
        &self,
        handle: &FcSandboxHandle,
        script: &str,
    ) -> Result<(), String> {
        let helper = &self.config.exec_helper;
        if !Path::new(helper).is_file() {
            return Err(format!(
                "fc exec helper not found at {} (set CLAW_FC_EXEC_HELPER)",
                helper.display()
            ));
        }

        let payload = json!({
            "op": "run_sh",
            "api_key": self.config.api_key,
            "domain": self.config.domain,
            "api_url": self.config.api_url,
            "sandbox_id": handle.sandbox_id,
            "script": script,
        });

        let out = tokio::process::Command::new("python3")
            .arg(helper)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn fc exec helper: {e}"))?;

        // Write stdin and collect output
        let mut child = out;
        if let Some(mut stdin) = child.stdin.take() {
            let bytes = serde_json::to_vec(&payload).map_err(|e| format!("exec payload: {e}"))?;
            stdin
                .write_all(&bytes)
                .await
                .map_err(|e| format!("fc exec stdin: {e}"))?;
        }
        let result = child
            .wait_with_output()
            .await
            .map_err(|e| format!("fc exec wait: {e}"))?;
        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let stdout = String::from_utf8_lossy(&result.stdout);
            warn!(
                target: "claw_fc_sandbox",
                sandbox_id = %handle.sandbox_id,
                stderr = %stderr,
                stdout = %stdout,
                "fc exec helper failed"
            );
            return Err(format!(
                "fc exec helper exit {}: {stderr}{stdout}",
                result.status
            ));
        }
        let stdout = String::from_utf8_lossy(&result.stdout);
        if let Ok(v) = serde_json::from_str::<Value>(&stdout) {
            if v.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
                return Ok(());
            }
            if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
                return Err(err.to_string());
            }
        }
        Ok(())
    }

    /// Build volume mounts for OVS session when `CLAW_FC_NAS_VOLUME_NAME` is set.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttyd_host_format() {
        let cfg = FcSandboxConfig {
            api_key: "e2b_test".into(),
            api_url: "https://api.cn-beijing.e2b.fc.aliyuncs.com".into(),
            domain: "cn-beijing.e2b.fc.aliyuncs.com".into(),
            template: "code-interpreter-v1".into(),
            sandbox_timeout_secs: 300,
            nas_server: None,
            nas_export: None,
            nas_volume_name: None,
            exec_helper: "deploy/fc-sandbox/fc_exec.py".into(),
            ttyd_port: 7681,
        };
        let c = FcSandboxClient::new(cfg);
        assert_eq!(
            c.ttyd_public_host("sbx-abc", "cn-beijing.e2b.fc.aliyuncs.com"),
            "7681-sbx-abc.cn-beijing.e2b.fc.aliyuncs.com"
        );
    }
}
