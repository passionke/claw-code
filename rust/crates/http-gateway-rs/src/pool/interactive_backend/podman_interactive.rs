//! Podman pool interactive backend (legacy `InteractiveSessionBind`). Author: kejiqing

use std::path::PathBuf;
use std::time::Duration;

use claw_sandbox_protocol::{GuestExecActor, InteractiveSessionBind, SlotLeaseOwner};

use crate::pool::{path_for_pool_acquire, PoolClients};

use super::{
    InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend, InteractiveSessionSpec,
    TtydConnectTarget,
};

pub struct PodmanInteractiveBackend {
    pool_clients: PoolClients,
    pool_id: String,
    work_root: PathBuf,
    pool_rpc_host_work_root: Option<PathBuf>,
}

impl PodmanInteractiveBackend {
    #[must_use]
    pub fn new(
        pool_clients: PoolClients,
        pool_id: String,
        work_root: PathBuf,
        pool_rpc_host_work_root: Option<PathBuf>,
    ) -> Self {
        Self {
            pool_clients,
            pool_id,
            work_root,
            pool_rpc_host_work_root,
        }
    }

    fn pick_ttyd_host_port(session_id: &str) -> Result<u16, String> {
        use std::net::TcpListener;
        let base: u16 = 37_681;
        let span: u16 = 8_000;
        let mut h: u32 = 0;
        for b in session_id.as_bytes() {
            h = h.wrapping_mul(31).wrapping_add(u32::from(*b));
        }
        for attempt in 0..64 {
            let port = base.wrapping_add((h.wrapping_add(attempt)) as u16 % span);
            if TcpListener::bind(("127.0.0.1", port)).is_ok() {
                return Ok(port);
            }
        }
        Err("no free ttyd host port in range".into())
    }
}

#[async_trait::async_trait]
impl InteractiveSandboxBackend for PodmanInteractiveBackend {
    async fn start_session(
        &self,
        spec: InteractiveSessionSpec,
    ) -> Result<InteractiveLease, String> {
        let sandbox = self
            .pool_clients
            .sandbox_rpc_client()
            .ok_or_else(|| "sandbox RPC client unavailable".to_string())?;

        let isolation = spec.sandbox_isolation;

        let proj_abs = std::fs::canonicalize(&spec.proj_home)
            .map_err(|e| format!("canonicalize proj home: {e}"))?;
        let session_abs = std::fs::canonicalize(&spec.session_home)
            .map_err(|e| format!("canonicalize session dir: {e}"))?;
        let ttyd_port = Self::pick_ttyd_host_port(&spec.session_id)?;

        let bind = InteractiveSessionBind {
            proj_home_host: path_for_pool_acquire(
                &proj_abs,
                &self.work_root,
                self.pool_rpc_host_work_root.as_deref(),
            )
            .to_string_lossy()
            .into_owned(),
            session_host_root: path_for_pool_acquire(
                &session_abs,
                &self.work_root,
                self.pool_rpc_host_work_root.as_deref(),
            )
            .to_string_lossy()
            .into_owned(),
            ttyd_host_port: ttyd_port,
            proj_home_readonly: !spec.ovs_mode,
        };

        let lease = sandbox
            .acquire(
                Duration::from_secs(45),
                isolation,
                Some(bind),
                Some(SlotLeaseOwner::Terminal {
                    session_id: spec.session_id.clone(),
                    proj_id: spec.proj_id,
                }),
            )
            .await?;

        let slot_index = lease.slot_index;
        if let Err(e) = sandbox
            .guest_exec_sh(
                slot_index,
                spec.start_ttyd_script,
                GuestExecActor::SlotWorker,
            )
            .await
        {
            let _ = sandbox.release(slot_index).await;
            return Err(format!("start ttyd in worker: {e}"));
        }

        let ttyd_port = lease.ttyd_host_port.unwrap_or(ttyd_port);
        let connect_host = std::env::var("CLAW_TERMINAL_TTYD_CONNECT_HOST")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());

        Ok(InteractiveLease {
            backend: InteractiveBackendKind::Podman,
            slot_index,
            worker_name: lease.worker_name,
            pool_id: self.pool_id.clone(),
            fc_sandbox_id: None,
            ttyd: TtydConnectTarget::loopback(ttyd_port, &connect_host),
        })
    }

    async fn stop_session(&self, lease: &InteractiveLease) -> Result<(), String> {
        if lease.backend != InteractiveBackendKind::Podman {
            return Err("podman stop called on non-podman lease".into());
        }
        let sandbox = self
            .pool_clients
            .sandbox_rpc_client()
            .ok_or_else(|| "sandbox RPC client unavailable".to_string())?;
        let _ = sandbox
            .guest_exec_sh(
                lease.slot_index,
                "pkill -f 'ttyd.*7681' 2>/dev/null || true",
                GuestExecActor::SlotWorker,
            )
            .await;
        sandbox.release(lease.slot_index).await
    }
}
