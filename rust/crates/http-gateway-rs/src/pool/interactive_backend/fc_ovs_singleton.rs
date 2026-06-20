//! FC OVS singleton — one openvscode-server sandbox per Gateway cluster. Author: kejiqing

use std::sync::Arc;

use claw_fc_sandbox_client::{FcSandboxClient, FcSandboxHandle};
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::fc_interactive_materialize::{
    self_hosted_nas_root_mount_sh, start_ovs_server_sh, OVS_WORKSPACE_ROOT,
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
    inner: Mutex<Option<OvsRuntime>>,
}

impl FcOvsSingleton {
    #[must_use]
    pub fn new(client: Arc<FcSandboxClient>) -> Self {
        let cluster_id = std::env::var("CLAW_CLUSTER_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "default".into());
        Self {
            client,
            cluster_id,
            inner: Mutex::new(None),
        }
    }

    /// Ensure OVS sandbox is running; returns public base URL (`…/ovs` without trailing folder).
    pub async fn ensure(&self) -> Result<String, String> {
        if let Some(rt) = self.inner.lock().await.clone() {
            if self.probe_local(&rt.handle).await.is_ok() {
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
        let mut script = String::from("set -e\n");
        let cfg = self.client.config();
        if cfg.is_self_hosted() {
            script.push_str(&self_hosted_nas_root_mount_sh(
                cfg.nas_server.as_deref().unwrap_or("10.8.0.8"),
                cfg.nas_export.as_deref().unwrap_or("/"),
            ));
        }
        script.push('\n');
        script.push_str(&start_ovs_server_sh(cfg.ovs_port));
        if let Err(e) = self.client.exec_shell_script(&handle, &script).await {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!("fc ovs singleton start: {e}"));
        }
        let base_url = self.client.ovs_public_base_url(&handle);
        info!(
            target: "claw_fc_ovs",
            sandbox_id = %handle.sandbox_id,
            %base_url,
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
            "{}/?folder={OVS_WORKSPACE_ROOT}/proj_{proj_id}/home",
            base_url.trim_end_matches('/')
        )
    }

    #[must_use]
    pub fn workspace_folder_path(proj_id: i64) -> String {
        format!("{OVS_WORKSPACE_ROOT}/proj_{proj_id}/home")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_folder_url_format() {
        let url = FcOvsSingleton::workspace_folder_url("http://3000-sbx.10.8.0.9/ovs", 2);
        assert!(url.contains("proj_2/home"));
        assert!(url.starts_with("http://3000-sbx.10.8.0.9/ovs/?folder="));
    }
}
