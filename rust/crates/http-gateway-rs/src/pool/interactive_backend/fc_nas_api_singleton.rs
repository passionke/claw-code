//! FC NAS API singleton — project resource writes via e2b sandbox HTTP. Author: kejiqing

use std::sync::Arc;

use claw_fc_sandbox_client::FcSandboxClient;
use claw_fc_sandbox_client::FcSandboxHandle;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::cluster_identity::gateway_cluster_id;

struct NasApiRuntime {
    handle: FcSandboxHandle,
    api_base_url: String,
}

/// One NAS API singleton per gateway process (persistent e2b sandbox).
pub struct FcNasApiSingleton {
    client: Arc<FcSandboxClient>,
    inner: Mutex<Option<NasApiRuntime>>,
    api_port: u16,
}

impl FcNasApiSingleton {
    #[must_use]
    pub fn new(client: Arc<FcSandboxClient>) -> Self {
        let api_port = std::env::var("CLAW_FC_NAS_API_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8090);
        Self {
            client,
            inner: Mutex::new(None),
            api_port,
        }
    }

    fn api_public_base(&self, handle: &FcSandboxHandle) -> String {
        let scheme = if self.client.config().is_self_hosted() {
            "http"
        } else {
            "https"
        };
        let host = format!(
            "{}-{}.{}",
            self.api_port, handle.sandbox_id, handle.sandbox_domain
        );
        format!("{scheme}://{host}")
    }

    pub async fn ensure_running(&self) -> Result<String, String> {
        if let Some(rt) = self.inner.lock().await.as_ref() {
            return Ok(rt.api_base_url.clone());
        }
        let cluster_id = gateway_cluster_id()?;
        let handle = self
            .client
            .create_nas_api_singleton_sandbox(&cluster_id)
            .await?;
        let internal_token = std::env::var("CLAW_GATEWAY_INTERNAL_TOKEN").unwrap_or_default();
        let script = super::fc_interactive_materialize::start_nas_api_server_sh(
            self.api_port,
            &internal_token,
        );
        if let Err(e) = self.client.exec_shell_script(&handle, &script).await {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!("fc nas-api singleton start: {e}"));
        }
        self.client
            .set_sandbox_timeout(
                &handle.sandbox_id,
                self.client.config().observe_sandbox_timeout_secs,
            )
            .await?;
        let api_base_url = self.api_public_base(&handle);
        info!(
            target: "claw_fc_nas_api",
            sandbox_id = %handle.sandbox_id,
            %api_base_url,
            "nas-api singleton created"
        );
        *self.inner.lock().await = Some(NasApiRuntime {
            handle,
            api_base_url: api_base_url.clone(),
        });
        Ok(api_base_url)
    }

    /// Gateway shutdown: detach only; sandbox survives across restarts.
    pub async fn shutdown(&self) {
        if self.inner.lock().await.take().is_some() {
            info!(target: "claw_fc_nas_api", "nas-api singleton detached");
        }
    }

    pub async fn reset(&self) -> Result<String, String> {
        if let Some(rt) = self.inner.lock().await.take() {
            if let Err(e) = self.client.kill_sandbox(&rt.handle.sandbox_id).await {
                warn!(
                    target: "claw_fc_nas_api",
                    sandbox_id = %rt.handle.sandbox_id,
                    error = %e,
                    "reset kill failed"
                );
            }
        }
        self.ensure_running().await
    }
}
