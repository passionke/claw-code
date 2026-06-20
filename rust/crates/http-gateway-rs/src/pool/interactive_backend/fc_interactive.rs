//! FC cloud sandbox interactive backend (proj-bound warm pool). Author: kejiqing

use std::sync::Arc;

use claw_fc_sandbox_client::FcSandboxClient;
use tracing::warn;

use super::fc_interactive_materialize::{self_hosted_proj_mount_sh, self_hosted_session_mount_sh};
use super::fc_warm_pool::FcProjWarmPool;
use super::{
    InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend, InteractiveSessionSpec,
    TtydConnectTarget,
};

pub struct FcInteractiveBackend {
    client: Arc<FcSandboxClient>,
    pool_id: String,
    warm: Arc<FcProjWarmPool>,
}

impl FcInteractiveBackend {
    #[must_use]
    pub fn new(client: Arc<FcSandboxClient>, pool_id: String) -> Self {
        let warm = Arc::new(FcProjWarmPool::from_env(Arc::clone(&client)));
        Self {
            client,
            pool_id,
            warm,
        }
    }

    pub async fn bind_session_db(&self, db: Arc<crate::session_db::GatewaySessionDb>) {
        self.warm.bind_session_db(db).await;
    }

    #[must_use]
    pub fn warm_pool(&self) -> &FcProjWarmPool {
        &self.warm
    }

    fn ttyd_target(&self, handle: &claw_fc_sandbox_client::FcSandboxHandle) -> TtydConnectTarget {
        if handle.ttyd_use_tls {
            return TtydConnectTarget::fc_public(handle.ttyd_public_host.clone());
        }
        let cfg = self.client.config();
        // Self-hosted: published ports (7681-ttyd, 3000-ovs) route via host :80 + Host header.
        // CLAW_E2B_SANDBOX_URL :3002 is envd/exec only — not the traffic proxy (see FC-OVS-E2E-FAILURES.md F9).
        let traffic_host = cfg
            .sandbox_url
            .as_deref()
            .map(parse_proxy_base)
            .map(|(h, _)| h)
            .unwrap_or_else(|| cfg.domain.clone());
        let traffic_port = std::env::var("CLAW_FC_TRAFFIC_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(80);
        TtydConnectTarget::e2b_self_hosted_proxy(
            traffic_host,
            traffic_port,
            handle.ttyd_public_host.clone(),
            handle.traffic_access_token.clone(),
        )
    }

    fn session_attach_script(&self, spec: &InteractiveSessionSpec) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push("set -e".to_string());
        let cfg = self.client.config();
        if cfg.is_self_hosted() {
            parts.push(self_hosted_session_mount_sh(
                &spec.session_id,
                spec.proj_id,
                cfg.nas_server.as_deref().unwrap_or("10.8.0.8"),
                cfg.nas_export.as_deref().unwrap_or("/"),
            ));
        }
        if let Some(ref attach) = spec.fc_session_attach_script {
            parts.push(attach.clone());
        }
        parts.push(spec.start_ttyd_script.to_string());
        parts.join("\n")
    }

    /// Cold path when warm pool is exhausted (one-shot sandbox; killed on stop).
    async fn start_session_cold(
        &self,
        spec: &InteractiveSessionSpec,
    ) -> Result<InteractiveLease, String> {
        warn!(
            target: "claw_fc_warm_pool",
            proj_id = spec.proj_id,
            session_id = %spec.session_id,
            "warm pool unavailable; cold create"
        );
        let handle = self
            .client
            .create_sandbox(&spec.session_id, spec.proj_id, true)
            .await?;
        let mut script = String::from("set -e\n");
        let cfg = self.client.config();
        if cfg.is_self_hosted() {
            script.push_str(&self_hosted_proj_mount_sh(
                spec.proj_id,
                cfg.nas_server.as_deref().unwrap_or("10.8.0.8"),
                cfg.nas_export.as_deref().unwrap_or("/"),
            ));
            script.push_str(&self_hosted_session_mount_sh(
                &spec.session_id,
                spec.proj_id,
                cfg.nas_server.as_deref().unwrap_or("10.8.0.8"),
                cfg.nas_export.as_deref().unwrap_or("/"),
            ));
        }
        if let Some(ref bake) = spec.fc_proj_bake_script {
            script.push('\n');
            script.push_str(bake);
        }
        if let Some(ref attach) = spec.fc_session_attach_script {
            script.push('\n');
            script.push_str(attach);
        }
        script.push('\n');
        script.push_str(spec.start_ttyd_script);
        if let Err(e) = self.client.exec_shell_script(&handle, &script).await {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!("fc cold start: {e}"));
        }
        Ok(InteractiveLease {
            backend: InteractiveBackendKind::Fc,
            slot_index: 0,
            worker_name: Some(format!("fc:{}", handle.sandbox_id)),
            pool_id: self.pool_id.clone(),
            fc_sandbox_id: Some(handle.sandbox_id.clone()),
            fc_warm_slot: None,
            fc_warm_proj_id: None,
            ttyd: self.ttyd_target(&handle),
        })
    }

    /// Gateway shutdown: release all warm-pool sandboxes.
    pub async fn shutdown_all(&self) {
        self.warm.shutdown_all().await;
    }
}

fn parse_proxy_base(url: &str) -> (String, u16) {
    let trimmed = url.trim().trim_end_matches('/');
    let no_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    if let Some((host, port)) = no_scheme.rsplit_once(':') {
        if let Ok(p) = port.parse::<u16>() {
            return (host.to_string(), p);
        }
    }
    (no_scheme.to_string(), 3002)
}

#[async_trait::async_trait]
impl InteractiveSandboxBackend for FcInteractiveBackend {
    async fn start_session(
        &self,
        spec: InteractiveSessionSpec,
    ) -> Result<InteractiveLease, String> {
        let attach_script = self.session_attach_script(&spec);
        match self.warm.acquire(spec.proj_id).await {
            Ok((handle, slot_index, _pooled)) => {
                self.client.touch_sandbox_lease(&handle.sandbox_id).await?;
                if let Err(e) = self.client.exec_shell_script(&handle, &attach_script).await {
                    let _ = self.warm.release(slot_index).await;
                    return Err(format!("fc warm attach session: {e}"));
                }
                Ok(InteractiveLease {
                    backend: InteractiveBackendKind::Fc,
                    slot_index,
                    worker_name: Some(format!("fc:{}", handle.sandbox_id)),
                    pool_id: self.pool_id.clone(),
                    fc_sandbox_id: Some(handle.sandbox_id.clone()),
                    fc_warm_slot: Some(slot_index),
                    fc_warm_proj_id: Some(spec.proj_id),
                    ttyd: self.ttyd_target(&handle),
                })
            }
            Err(e) => {
                warn!(
                    target: "claw_fc_warm_pool",
                    proj_id = spec.proj_id,
                    error = %e,
                    "acquire failed; trying cold start"
                );
                self.start_session_cold(&spec).await
            }
        }
    }

    async fn stop_session(&self, lease: &InteractiveLease) -> Result<(), String> {
        if lease.backend != InteractiveBackendKind::Fc {
            return Err("fc stop called on non-fc lease".into());
        }
        if let Some(slot_index) = lease.fc_warm_slot {
            let result = self.warm.release(slot_index).await;
            if let Some(proj_id) = lease.fc_warm_proj_id {
                FcProjWarmPool::schedule_ensure_warm(&self.warm, proj_id);
            }
            return result;
        }
        let sandbox_id = lease
            .fc_sandbox_id
            .as_deref()
            .ok_or_else(|| "fc lease missing sandbox_id".to_string())?;
        self.client.kill_sandbox(sandbox_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_base_host_port() {
        assert_eq!(
            parse_proxy_base("http://10.8.0.9:3002"),
            ("10.8.0.9".into(), 3002)
        );
    }
}
