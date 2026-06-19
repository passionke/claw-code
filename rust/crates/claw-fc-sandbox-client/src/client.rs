//! E2B-compatible REST client for Alibaba FC cloud sandbox. Author: kejiqing

use std::collections::BTreeMap;
use std::path::Path;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;
use tracing::{debug, warn};

use crate::config::FcSandboxConfig;
use crate::types::{CreateSandboxResponse, FcExecOutcome, FcSandboxHandle, FcSandboxVolumeMount};

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
        Ok(FcSandboxHandle {
            sandbox_id: parsed.sandbox_id,
            sandbox_domain,
            envd_access_token: parsed.envd_access_token,
            traffic_access_token: parsed.traffic_access_token,
            ttyd_public_host,
            ttyd_use_tls: !self.config.is_self_hosted(),
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

    /// Run `claw gateway-solve-once` inside an FC sandbox.
    pub async fn exec_gateway_solve_once(
        &self,
        sandbox_id: &str,
        task_rel_under_root: &str,
        claw_bin: &str,
        env: BTreeMap<String, String>,
    ) -> Result<FcExecOutcome, String> {
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
        Self::run_exec_helper(&self.config.exec_helper, &payload).await
    }

    /// Run a shell script inside the sandbox via `deploy/fc-sandbox/fc_exec.py` (envd gRPC).
    pub async fn exec_shell_script(
        &self,
        handle: &FcSandboxHandle,
        script: &str,
    ) -> Result<(), String> {
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
        Self::run_exec_helper(&self.config.exec_helper, &payload)
            .await
            .map(|_| ())
    }

    async fn run_exec_helper(helper: &Path, payload: &Value) -> Result<FcExecOutcome, String> {
        if !helper.is_file() {
            return Err(format!(
                "fc exec helper not found at {} (set CLAW_FC_EXEC_HELPER)",
                helper.display()
            ));
        }

        let out = tokio::process::Command::new("python3")
            .arg(helper)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn fc exec helper: {e}"))?;

        let mut child = out;
        if let Some(mut stdin) = child.stdin.take() {
            let bytes = serde_json::to_vec(payload).map_err(|e| format!("exec payload: {e}"))?;
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
            if v.get("ok").and_then(Value::as_bool) == Some(true) {
                if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
                    return Err(err.to_string());
                }
                if let Some(exit_code) = v.get("exit_code").and_then(Value::as_i64) {
                    return Ok(FcExecOutcome {
                        exit_code: i32::try_from(exit_code).unwrap_or(-1),
                        stdout: v
                            .get("stdout")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                        stderr: v
                            .get("stderr")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string(),
                    });
                }
                return Ok(FcExecOutcome {
                    exit_code: 0,
                    stdout: v
                        .get("stdout")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string(),
                    stderr: String::new(),
                });
            }
            if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
                return Err(err.to_string());
            }
        }
        Ok(FcExecOutcome {
            exit_code: 0,
            stdout: stdout.into_owned(),
            stderr: String::new(),
        })
    }

    /// Dynamic NAS at sandbox create (`nasConfig`); Aliyun FC only.
    fn nas_config_json(&self, session_id: &str, proj_id: i64, ovs_mode: bool) -> Option<Value> {
        if self.config.is_self_hosted() {
            return None;
        }
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
        };
        let c = FcSandboxClient::new(cfg);
        assert_eq!(
            c.ttyd_public_host("sbx-abc", "cn-beijing.e2b.fc.aliyuncs.com"),
            "7681-sbx-abc.cn-beijing.e2b.fc.aliyuncs.com"
        );
    }
}
