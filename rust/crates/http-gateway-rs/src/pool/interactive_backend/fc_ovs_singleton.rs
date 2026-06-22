//! FC OVS singleton — one openvscode-server sandbox per Gateway cluster. Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;

use claw_fc_sandbox_client::{FcSandboxClient, FcSandboxHandle};
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::fc_interactive_materialize::start_ovs_server_sh;
use super::fc_ovs_claw_vscode::{
    ensure_claw_vscode_if_needed, ensure_claw_vscode_on_ovs, probe_claw_vscode_installed,
};

#[derive(Clone)]
struct OvsRuntime {
    handle: FcSandboxHandle,
    base_url: String,
}

/// Lazy singleton OVS sandbox for `CLAW_OVS_BACKEND=fc`.
pub struct FcOvsSingleton {
    client: Arc<FcSandboxClient>,
    cluster_id: String,
    nas_root: PathBuf,
    gateway_port: u16,
    inner: Mutex<Option<OvsRuntime>>,
}

impl FcOvsSingleton {
    #[must_use]
    pub fn new(client: Arc<FcSandboxClient>, nas_root: PathBuf, gateway_port: u16) -> Self {
        let cluster_id = std::env::var("CLAW_CLUSTER_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "default".into());
        Self {
            client,
            cluster_id,
            nas_root,
            gateway_port,
            inner: Mutex::new(None),
        }
    }

    /// Ensure OVS sandbox is running; returns public base URL (`…/ovs` without trailing folder).
    pub async fn ensure(&self) -> Result<String, String> {
        if let Some(rt) = self.inner.lock().await.clone() {
            if self.probe_local(&rt.handle).await.is_ok() {
                ensure_claw_vscode_if_needed(
                    &self.client,
                    &rt.handle,
                    self.client.config().ovs_port,
                    &self.nas_root,
                    self.gateway_port,
                )
                .await;
                return Ok(rt.base_url.clone());
            }
            warn!(
                target: "claw_fc_ovs",
                sandbox_id = %rt.handle.sandbox_id,
                "ovs singleton unhealthy; recreating"
            );
            let _ = self.client.kill_sandbox(&rt.handle.sandbox_id).await;
            *self.inner.lock().await = None;
        }
        self.create_and_start().await
    }

    async fn probe_local(&self, handle: &FcSandboxHandle) -> Result<(), String> {
        let port = self.client.config().ovs_port;
        let script = format!("curl -fsS -m 5 http://127.0.0.1:{port}/ovs/ >/dev/null");
        self.client.exec_shell_script(handle, &script).await?;
        Ok(())
    }

    async fn create_and_start(&self) -> Result<String, String> {
        let handle = self
            .client
            .create_ovs_singleton_sandbox(&self.cluster_id)
            .await?;
        let ovs_port = self.client.config().ovs_port;
        let mut script = String::from("set -e\n");
        script.push_str(&start_ovs_server_sh(ovs_port));
        if let Err(e) = self.client.exec_shell_script(&handle, &script).await {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!("fc ovs singleton start: {e}"));
        }
        match ensure_claw_vscode_on_ovs(
            &self.client,
            &handle,
            ovs_port,
            &self.nas_root,
            self.gateway_port,
        )
        .await
        {
            Ok(()) => {}
            Err(e) => {
                warn!(
                    target: "claw_fc_ovs",
                    sandbox_id = %handle.sandbox_id,
                    error = %e,
                    "claw-vscode install after ovs start failed"
                );
            }
        }
        let base_url = self.client.ovs_public_base_url(&handle);
        let claw_vscode_ok = probe_claw_vscode_installed(&self.client, &handle)
            .await
            .is_ok();
        info!(
            target: "claw_fc_ovs",
            sandbox_id = %handle.sandbox_id,
            %base_url,
            claw_vscode = claw_vscode_ok,
            "ovs singleton ready"
        );
        *self.inner.lock().await = Some(OvsRuntime {
            handle,
            base_url: base_url.clone(),
        });
        Ok(base_url)
    }

    /// Gateway shutdown: kill OVS singleton sandbox.
    pub async fn shutdown(&self) {
        let rt = self.inner.lock().await.take();
        if let Some(rt) = rt {
            let sid = rt.handle.sandbox_id.clone();
            if let Err(e) = self.client.kill_sandbox(&sid).await {
                warn!(target: "claw_fc_ovs", sandbox_id = %sid, error = %e, "ovs shutdown kill failed");
            } else {
                info!(target: "claw_fc_ovs", sandbox_id = %sid, "ovs singleton killed");
            }
        }
    }

    /// Folder URL for a project inside the singleton OVS.
    #[must_use]
    pub fn workspace_folder_url(base_url: &str, proj_id: i64) -> String {
        format!(
            "{}?folder={}/proj_{proj_id}/home",
            base_url.trim_end_matches('/'),
            super::fc_interactive_materialize::OVS_WORKSPACE_ROOT
        )
    }

    #[must_use]
    pub fn workspace_folder_path(proj_id: i64) -> String {
        format!(
            "{}/proj_{proj_id}/home",
            super::fc_interactive_materialize::OVS_WORKSPACE_ROOT
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_folder_url_format() {
        let url = FcOvsSingleton::workspace_folder_url("http://3000-sbx_abc.supone.top/ovs", 2);
        assert!(url.contains("proj_2/home"));
        assert!(url.starts_with("http://3000-sbx_abc.supone.top/ovs?folder="));
    }

    #[test]
    fn ovs_start_script_sets_home_and_ovs_bin() {
        let sh = super::super::fc_interactive_materialize::start_ovs_server_sh(3000);
        assert!(sh.contains("export HOME=/opt/claw-ovs/home"));
        assert!(sh.contains("/home/.openvscode-server/bin/openvscode-server"));
        assert!(
            !sh.contains("--default-folder="),
            "FC OVS must not set --default-folder (breaks ?folder=proj_N/home; see INCIDENT-2026-06-18)"
        );
    }
}
