//! FC session-observe singleton — **Live 浏览**（读 NAS traces），不负责 solve LLM proxy。
//! Worker tap（`fc_worker_tap`）负责 proxy + trace 写入；本 singleton 只跑 `--tap-live`。
//! Author: kejiqing

use std::sync::Arc;

use claw_fc_sandbox_client::{FcSandboxClient, FcSandboxHandle};
use tokio::sync::Mutex;
use tracing::{info, warn};

use super::fc_interactive_materialize::{start_observe_server_sh, OVS_WORKSPACE_ROOT};
use super::fc_worker_tap::fc_worker_tap_database_url;

#[derive(Clone)]
struct ObserveRuntime {
    handle: FcSandboxHandle,
    live_base_url: String,
}

/// Lazy singleton session-observe sandbox for `CLAW_INTERACTIVE_BACKEND=fc`.
pub struct FcSessionObserveSingleton {
    client: Arc<FcSandboxClient>,
    cluster_id: String,
    inner: Mutex<Option<ObserveRuntime>>,
}

impl FcSessionObserveSingleton {
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

    /// Cached Live base URL for Admin (no probe / recreate on hot path).
    pub async fn cached_live_base_url(&self) -> Option<String> {
        self.inner
            .lock()
            .await
            .as_ref()
            .map(|rt| rt.live_base_url.clone())
    }

    /// Ensure observe sandbox is running; returns public Live base URL.
    pub async fn ensure(&self) -> Result<String, String> {
        if let Some(rt) = self.inner.lock().await.clone() {
            if self.probe_local(&rt.handle).await.is_ok() {
                return Ok(rt.live_base_url.clone());
            }
            warn!(
                target: "claw_fc_observe",
                sandbox_id = %rt.handle.sandbox_id,
                "observe singleton unhealthy; recreating"
            );
            let _ = self.client.kill_sandbox(&rt.handle.sandbox_id).await;
            *self.inner.lock().await = None;
        }
        self.create_and_start().await
    }

    async fn probe_local(&self, handle: &FcSandboxHandle) -> Result<(), String> {
        let port = self.client.config().observe_live_port;
        let script = format!("curl -fsS -m 5 http://127.0.0.1:{port}/ >/dev/null");
        self.client.exec_shell_script(handle, &script).await?;
        Ok(())
    }

    async fn create_and_start(&self) -> Result<String, String> {
        let handle = self
            .client
            .create_observe_singleton_sandbox(&self.cluster_id)
            .await?;
        let db_url = fc_worker_tap_database_url()?;
        let mut script = String::from("set -e\n");
        script.push_str(&start_observe_server_sh(
            self.client.config().observe_live_port,
            &self.cluster_id,
            &db_url,
        ));
        if let Err(e) = self.client.exec_shell_script(&handle, &script).await {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!("fc observe singleton start: {e}"));
        }
        let live_base_url = self.client.observe_public_live_base_url(&handle);
        info!(
            target: "claw_fc_observe",
            sandbox_id = %handle.sandbox_id,
            %live_base_url,
            nas_root = %OVS_WORKSPACE_ROOT,
            "observe singleton ready"
        );
        *self.inner.lock().await = Some(ObserveRuntime {
            handle,
            live_base_url: live_base_url.clone(),
        });
        Ok(live_base_url)
    }

    /// Gateway shutdown: kill observe singleton sandbox.
    pub async fn shutdown(&self) {
        let rt = self.inner.lock().await.take();
        if let Some(rt) = rt {
            let sid = rt.handle.sandbox_id.clone();
            if let Err(e) = self.client.kill_sandbox(&sid).await {
                warn!(target: "claw_fc_observe", sandbox_id = %sid, error = %e, "observe shutdown kill failed");
            } else {
                info!(target: "claw_fc_observe", sandbox_id = %sid, "observe singleton killed");
            }
        }
    }

    /// Admin session link template (`{sessionId}` placeholder).
    #[must_use]
    pub fn live_session_url_template(live_base_url: &str) -> String {
        crate::gateway_claw_tap_settings::live_session_viewer_url_template(live_base_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_session_url_template_format() {
        let t =
            FcSessionObserveSingleton::live_session_url_template("http://3000-sbx_abc.supone.top");
        assert!(t.contains("session={sessionId}"));
        assert!(t.contains("?session="));
    }

    #[test]
    fn observe_start_script_uses_tap_live_cli() {
        let sh = super::super::fc_interactive_materialize::start_observe_server_sh(
            3000,
            "local-dev",
            "postgres://u:p@10.8.0.10:5433/claw_gateway",
        );
        assert!(sh.contains("--tap-no-launch"));
        assert!(sh.contains("--tap-live"));
        assert!(sh.contains("--tap-live-port 3000"));
        assert!(!sh.contains("CLAUDE_TAP_LIVE_PREFIX_PATH"));
        assert!(sh.contains("/claw_ws/tap-traces"));
        assert!(!sh.contains("claude-tap serve"));
    }
}
