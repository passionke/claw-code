//! FC cloud sandbox interactive backend (proj-bound warm pool). Author: kejiqing

use std::sync::Arc;

use claw_fc_sandbox_client::FcSandboxClient;
use tracing::warn;

use super::fc_warm_pool::FcProjWarmPool;
use super::{
    InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend, InteractiveSessionSpec,
    TtydConnectTarget,
};
use crate::pool::fc_nas_layout::{
    allocate_worker_id, ensure_fc_proj_nas_roots, ensure_worker_root_on_nas, fc_nas_layout_active,
    link_session_to_worker, nas_host_root, unlink_session_symlink,
};

pub struct FcInteractiveBackend {
    client: Arc<FcSandboxClient>,
    pool_id: String,
    nas_root: std::path::PathBuf,
    warm: Arc<FcProjWarmPool>,
    runtime_bin: String,
}

impl FcInteractiveBackend {
    #[must_use]
    pub fn new(
        client: Arc<FcSandboxClient>,
        pool_id: String,
        work_root: std::path::PathBuf,
        pool_rpc_host_work_root: Option<std::path::PathBuf>,
    ) -> Self {
        let nas_root = nas_host_root(&work_root, pool_rpc_host_work_root.as_deref());
        let warm = Arc::new(FcProjWarmPool::from_env(
            Arc::clone(&client),
            nas_root.clone(),
        ));
        let runtime_bin =
            std::env::var("CLAW_CONTAINER_RUNTIME").unwrap_or_else(|_| "podman".into());
        Self {
            client,
            pool_id,
            nas_root,
            warm,
            runtime_bin,
        }
    }

    pub async fn bind_session_db(&self, db: Arc<crate::session_db::GatewaySessionDb>) {
        self.warm.bind_session_db(db).await;
    }

    #[must_use]
    pub fn warm_pool(&self) -> &FcProjWarmPool {
        &self.warm
    }

    async fn link_session(
        &self,
        proj_id: i64,
        session_segment: &str,
        worker_id: &str,
    ) -> Result<(), String> {
        if !fc_nas_layout_active(&self.nas_root) {
            return Ok(());
        }
        ensure_worker_root_on_nas(&self.runtime_bin, &self.nas_root, proj_id, worker_id).await?;
        link_session_to_worker(&self.nas_root, proj_id, session_segment, worker_id).await?;
        Ok(())
    }

    async fn unlink_session(&self, proj_id: i64, session_segment: &str) -> Result<(), String> {
        if !fc_nas_layout_active(&self.nas_root) {
            return Ok(());
        }
        unlink_session_symlink(&self.nas_root, proj_id, session_segment).await
    }

    fn ttyd_target(&self, handle: &claw_fc_sandbox_client::FcSandboxHandle) -> TtydConnectTarget {
        if handle.ttyd_use_tls {
            return TtydConnectTarget::fc_public(handle.ttyd_public_host.clone());
        }
        let cfg = self.client.config();
        let traffic_host = cfg
            .sandbox_url
            .as_deref()
            .map(parse_proxy_base)
            .map(|(h, _)| h)
            .unwrap_or_else(|| cfg.domain.clone());
        let traffic_port = crate::gateway_fc_observe_proxy::fc_traffic_proxy_port();
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
        let worker_id = allocate_worker_id();
        if fc_nas_layout_active(&self.nas_root) {
            ensure_fc_proj_nas_roots(&self.nas_root, spec.proj_id).await?;
            ensure_worker_root_on_nas(&self.runtime_bin, &self.nas_root, spec.proj_id, &worker_id)
                .await?;
        }
        let handle = self
            .client
            .create_sandbox(
                &spec.session_id,
                &spec.session_segment,
                spec.proj_id,
                true,
                &worker_id,
            )
            .await?;
        self.link_session(spec.proj_id, &spec.session_segment, &worker_id)
            .await?;
        let mut script = String::from("set -e\n");
        if let Some(ref bake) = spec.fc_proj_bake_script {
            script.push_str(bake);
            script.push('\n');
        }
        if let Some(ref attach) = spec.fc_session_attach_script {
            script.push_str(attach);
            script.push('\n');
        }
        script.push_str(&spec.start_ttyd_script);
        if let Err(e) = self.client.exec_shell_script(&handle, &script).await {
            let _ = self
                .unlink_session(spec.proj_id, &spec.session_segment)
                .await;
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
            fc_warm_proj_id: Some(spec.proj_id),
            fc_session_segment: Some(spec.session_segment.clone()),
            fc_worker_id: Some(worker_id),
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
            Ok((handle, slot_index, worker_id)) => {
                self.client.touch_sandbox_lease(&handle.sandbox_id).await?;
                if let Err(e) = self
                    .link_session(spec.proj_id, &spec.session_segment, &worker_id)
                    .await
                {
                    let _ = self.warm.release(slot_index, &spec.session_segment).await;
                    return Err(format!("fc warm link session: {e}"));
                }
                if let Err(e) = self.client.exec_shell_script(&handle, &attach_script).await {
                    let _ = self
                        .unlink_session(spec.proj_id, &spec.session_segment)
                        .await;
                    let _ = self.warm.release(slot_index, &spec.session_segment).await;
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
                    fc_session_segment: Some(spec.session_segment.clone()),
                    fc_worker_id: Some(worker_id),
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
            let segment = lease
                .fc_session_segment
                .as_deref()
                .ok_or_else(|| "fc warm release: missing session segment".to_string())?;
            let result = self.warm.release(slot_index, segment).await;
            if let Some(proj_id) = lease.fc_warm_proj_id {
                FcProjWarmPool::schedule_ensure_warm(&self.warm, proj_id);
            }
            return result;
        }
        if let (Some(proj_id), Some(segment)) =
            (lease.fc_warm_proj_id, lease.fc_session_segment.as_deref())
        {
            let _ = self.unlink_session(proj_id, segment).await;
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
