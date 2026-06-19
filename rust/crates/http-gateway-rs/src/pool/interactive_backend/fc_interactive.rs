//! FC cloud sandbox interactive backend. Author: kejiqing

use std::sync::Arc;

use claw_fc_sandbox_client::FcSandboxClient;

use super::{
    InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend, InteractiveSessionSpec,
    TtydConnectTarget,
};

pub struct FcInteractiveBackend {
    client: Arc<FcSandboxClient>,
    pool_id: String,
}

impl FcInteractiveBackend {
    #[must_use]
    pub fn new(client: Arc<FcSandboxClient>, pool_id: String) -> Self {
        Self { client, pool_id }
    }
}

#[async_trait::async_trait]
impl InteractiveSandboxBackend for FcInteractiveBackend {
    async fn start_session(
        &self,
        spec: InteractiveSessionSpec,
    ) -> Result<InteractiveLease, String> {
        let mounts = self.client.default_volume_mounts(spec.ovs_mode);
        let handle = self
            .client
            .create_sandbox(&spec.session_id, spec.proj_id, &mounts)
            .await?;

        if let Err(e) = self
            .client
            .exec_shell_script(&handle, spec.start_ttyd_script)
            .await
        {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!("fc start ttyd: {e}"));
        }

        Ok(InteractiveLease {
            backend: InteractiveBackendKind::Fc,
            slot_index: 0,
            worker_name: Some(format!("fc:{}", handle.sandbox_id)),
            pool_id: self.pool_id.clone(),
            fc_sandbox_id: Some(handle.sandbox_id.clone()),
            ttyd: TtydConnectTarget::fc_public(handle.ttyd_public_host),
        })
    }

    async fn stop_session(&self, lease: &InteractiveLease) -> Result<(), String> {
        if lease.backend != InteractiveBackendKind::Fc {
            return Err("fc stop called on non-fc lease".into());
        }
        let sandbox_id = lease
            .fc_sandbox_id
            .as_deref()
            .ok_or_else(|| "fc lease missing sandbox_id".to_string())?;
        self.client.kill_sandbox(sandbox_id).await
    }
}
