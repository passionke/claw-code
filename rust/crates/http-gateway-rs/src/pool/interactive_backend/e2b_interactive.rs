//! e2b cloud sandbox interactive backend (proj-bound worker registry). Author: kejiqing

use std::sync::Arc;

use claw_e2b_sandbox_client::E2bSandboxClient;

use super::{
    InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend, InteractiveSessionSpec,
    TtydConnectTarget,
};
use crate::pool::e2b_proj_worker_registry::E2bProjWorkerRegistry;
use crate::pool::NasLayoutBackend;

pub struct E2bInteractiveBackend {
    client: Arc<E2bSandboxClient>,
    pool_id: String,
    nas_layout: NasLayoutBackend,
    workers: Arc<E2bProjWorkerRegistry>,
}

impl E2bInteractiveBackend {
    #[must_use]
    pub fn new(
        client: Arc<E2bSandboxClient>,
        pool_id: String,
        nas_layout: NasLayoutBackend,
        workers: Arc<E2bProjWorkerRegistry>,
    ) -> Self {
        Self {
            client,
            pool_id,
            nas_layout,
            workers,
        }
    }

    #[allow(clippy::unused_async)]
    pub async fn bind_session_db(&self, _db: Arc<crate::session_db::GatewaySessionDb>) {
        // Bound via PoolClients::bind_session_db → e2b_workers.
    }

    #[must_use]
    pub fn worker_registry(&self) -> &E2bProjWorkerRegistry {
        &self.workers
    }

    async fn ensure_session(
        &self,
        proj_id: i64,
        session_segment: &str,
        worker_id: &str,
    ) -> Result<(), String> {
        self.nas_layout
            .ensure_worker_root(proj_id, worker_id)
            .await?;
        self.nas_layout
            .ensure_session_context(proj_id, session_segment, worker_id)
            .await
    }

    fn ttyd_target(&self, handle: &claw_e2b_sandbox_client::E2bSandboxHandle) -> TtydConnectTarget {
        if handle.ttyd_use_tls {
            return TtydConnectTarget::e2b_public(handle.ttyd_public_host.clone());
        }
        let cfg = self.client.config();
        let traffic_host = cfg
            .sandbox_url
            .as_deref()
            .map(parse_proxy_base)
            .map(|(h, _)| h)
            .unwrap_or_else(|| cfg.domain.clone());
        let traffic_port = crate::gateway_e2b_observe_proxy::e2b_traffic_proxy_port();
        TtydConnectTarget::e2b_self_hosted_proxy(
            traffic_host,
            traffic_port,
            handle.ttyd_public_host.clone(),
            handle.traffic_access_token.clone(),
        )
    }

    fn session_attach_script(spec: &InteractiveSessionSpec) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push("set -e".to_string());
        if let Some(ref attach) = spec.e2b_session_attach_script {
            parts.push(attach.clone());
        }
        parts.push(spec.start_ttyd_script.clone());
        parts.join("\n")
    }

    /// Gateway shutdown: workers survive on e2b (registry clears in-memory state only).
    pub async fn shutdown_all(&self) {
        self.workers.shutdown_all().await;
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
impl InteractiveSandboxBackend for E2bInteractiveBackend {
    async fn start_session(
        &self,
        spec: InteractiveSessionSpec,
    ) -> Result<InteractiveLease, String> {
        let attach_script = Self::session_attach_script(&spec);
        let (handle, worker_id) = self.workers.acquire(spec.proj_id).await?;
        if let Err(e) = self
            .ensure_session(spec.proj_id, &spec.session_segment, &worker_id)
            .await
        {
            self.workers.release(spec.proj_id).await;
            return Err(format!("fc ensure session: {e}"));
        }
        if let Err(e) = self
            .client
            .exec_shell_script(&handle, &attach_script, None)
            .await
        {
            self.workers.release(spec.proj_id).await;
            return Err(format!("fc attach session: {e}"));
        }
        Ok(InteractiveLease {
            backend: InteractiveBackendKind::E2b,
            slot_index: usize::try_from(spec.proj_id).unwrap_or(0),
            worker_name: Some(format!("e2b:{}", handle.sandbox_id)),
            pool_id: self.pool_id.clone(),
            e2b_sandbox_id: Some(handle.sandbox_id.clone()),
            e2b_warm_slot: Some(usize::try_from(spec.proj_id).unwrap_or(0)),
            e2b_warm_proj_id: Some(spec.proj_id),
            e2b_session_segment: Some(spec.session_segment.clone()),
            e2b_worker_id: Some(worker_id),
            ttyd: self.ttyd_target(&handle),
        })
    }

    async fn stop_session(&self, lease: &InteractiveLease) -> Result<(), String> {
        if lease.backend != InteractiveBackendKind::E2b {
            return Err("fc stop called on non-fc lease".into());
        }
        if let (Some(proj_id), Some(segment)) =
            (lease.e2b_warm_proj_id, lease.e2b_session_segment.as_deref())
        {
            let _ = segment;
            self.workers.release(proj_id).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_base_host_port() {
        assert_eq!(
            parse_proxy_base("http://10.8.0.1:3002"),
            ("10.8.0.1".into(), 3002)
        );
    }
}
