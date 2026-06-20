//! Gateway → claw-sandbox RPC + optional FC cloud sandbox per project. Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;

use claw_fc_sandbox_client::FcSandboxClient;
use claw_sandbox_client::SandboxRpcClient;

use crate::session_db::GatewaySessionDb;

use super::config::relaxed_worker_allowed_from_env;
use super::fc_orchestrated_pool::{FcOrchestratedPool, FC_POOL_ID};
use super::interactive_backend::{
    interactive_backend_is_fc, ovs_backend_is_fc, FcInteractiveBackend, FcOvsSingleton,
    InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend, PodmanInteractiveBackend,
};
use super::sandbox_orchestrator::SandboxOrchestratedPool;
use super::traits::PoolOps;
use super::worker_isolation::{
    default_worker_isolation_json, effective_mode, execution_backend_from_json, is_fc_sandbox_mode,
    mode_from_json, WorkerExecutionBackend, WorkerIsolationMode,
};
use super::LiveReportHub;

/// Pool routing: podman claw-sandbox + optional FC cloud sandbox. Author: kejiqing
#[derive(Clone)]
pub struct PoolClients {
    podman_pool: Arc<dyn PoolOps + Send + Sync>,
    fc_pool: Option<Arc<FcOrchestratedPool>>,
    pool_id: String,
    sandbox_pool: Option<Arc<SandboxOrchestratedPool>>,
    podman_interactive: Arc<PodmanInteractiveBackend>,
    fc_interactive: Option<Arc<FcInteractiveBackend>>,
    fc_ovs: Option<Arc<FcOvsSingleton>>,
}

impl PoolClients {
    #[must_use]
    pub fn from_env(
        live_report_hub: Arc<LiveReportHub>,
        work_root: PathBuf,
        fc_client: Option<Arc<FcSandboxClient>>,
        pool_rpc_host_work_root: Option<PathBuf>,
    ) -> Self {
        let sandbox_base = std::env::var("CLAW_SANDBOX_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                std::env::var("CLAW_POOL_HTTP_BASE")
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            });
        let sandbox_base = sandbox_base.unwrap_or_else(|| {
            eprintln!("http-gateway-rs: set CLAW_SANDBOX_URL or CLAW_POOL_HTTP_BASE");
            std::process::exit(1);
        });

        let pool_id = std::env::var("CLAW_POOL_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(crate::pool_registry::resolve_pool_id);

        let client = Arc::new(SandboxRpcClient::new(&sandbox_base));
        let orch = SandboxOrchestratedPool::new(
            client.clone(),
            work_root.clone(),
            pool_id.clone(),
            live_report_hub.clone(),
        );
        let sandbox_pool = Some(Arc::clone(&orch));
        let podman_pool: Arc<dyn PoolOps + Send + Sync> = orch;

        let fc_pool = fc_client.as_ref().map(|fc| {
            Arc::new(FcOrchestratedPool::new(
                Arc::clone(fc),
                work_root.clone(),
                live_report_hub,
            ))
        });
        let fc_interactive = fc_client
            .clone()
            .map(|fc| Arc::new(FcInteractiveBackend::new(fc, pool_id.clone())));
        if let Some(ref fc) = fc_client {
            FcSandboxClient::spawn_lease_ticker(Arc::clone(fc));
        }
        let fc_ovs = match (fc_client.as_ref(), ovs_backend_is_fc()) {
            (Some(fc), true) => Some(Arc::new(FcOvsSingleton::new(Arc::clone(fc)))),
            _ => None,
        };
        let podman_interactive = Arc::new(PodmanInteractiveBackend::new(
            client,
            pool_id.clone(),
            work_root,
            pool_rpc_host_work_root,
        ));

        Self {
            podman_pool,
            fc_pool,
            pool_id,
            sandbox_pool,
            podman_interactive,
            fc_interactive,
            fc_ovs,
        }
    }

    #[must_use]
    pub fn fc_interactive(&self) -> Option<&FcInteractiveBackend> {
        self.fc_interactive.as_deref()
    }

    #[must_use]
    pub fn fc_ovs_singleton(&self) -> Option<&FcOvsSingleton> {
        self.fc_ovs.as_deref()
    }

    #[must_use]
    pub fn fc_warm_pool(&self) -> Option<&super::interactive_backend::FcProjWarmPool> {
        self.fc_interactive.as_ref().map(|b| b.warm_pool())
    }

    /// Graceful shutdown: kill FC warm workers + OVS singleton (avoids e2b orphans on gateway restart).
    pub async fn shutdown_fc_sandboxes(&self) {
        if let Some(fc) = &self.fc_interactive {
            fc.shutdown_all().await;
        }
        if let Some(ovs) = &self.fc_ovs {
            ovs.shutdown().await;
        }
    }

    #[must_use]
    pub fn pool_id(&self) -> &str {
        &self.pool_id
    }

    #[must_use]
    pub fn client(&self) -> Arc<dyn PoolOps + Send + Sync> {
        Arc::clone(&self.podman_pool)
    }

    pub async fn worker_json_for_proj(db: &GatewaySessionDb, proj_id: i64) -> serde_json::Value {
        db.get_worker_isolation_json(proj_id)
            .await
            .unwrap_or_else(|_| default_worker_isolation_json())
    }

    pub async fn effective_mode_for_proj(
        db: &GatewaySessionDb,
        proj_id: i64,
    ) -> WorkerIsolationMode {
        let json = Self::worker_json_for_proj(db, proj_id).await;
        effective_mode(relaxed_worker_allowed_from_env(), &json)
    }

    pub async fn assert_proj_worker_isolation_supported(
        &self,
        db: &GatewaySessionDb,
        proj_id: i64,
    ) -> Result<(), String> {
        let json = Self::worker_json_for_proj(db, proj_id).await;
        if is_fc_sandbox_mode(&json) {
            if self.fc_pool.is_none() || self.fc_interactive.is_none() {
                return Err(
                    "proj worker_isolation_json.mode=sandbox but FC sandbox is not configured on gateway \
                     (set CLAW_FC_* / NAS_BASE_URL; FC template must have VPC for nasConfig)"
                        .into(),
                );
            }
            return Ok(());
        }
        if mode_from_json(&json) != WorkerIsolationMode::Relaxed {
            return Ok(());
        }
        if !relaxed_worker_allowed_from_env() {
            return Err(
                "proj worker_isolation_json.mode=relaxed but CLAW_ALLOW_RELAXED_WORKER=false on gateway; \
                 set CLAW_ALLOW_RELAXED_WORKER=true in repo .env and restart pool + gateway, or set proj to strict"
                    .into(),
            );
        }
        Ok(())
    }

    pub async fn pool_and_id_for_proj(
        &self,
        db: &GatewaySessionDb,
        proj_id: i64,
    ) -> Result<(Arc<dyn PoolOps + Send + Sync>, String), String> {
        let json = Self::worker_json_for_proj(db, proj_id).await;
        match execution_backend_from_json(&json) {
            WorkerExecutionBackend::FcSandbox => {
                let fc = self
                    .fc_pool
                    .as_ref()
                    .ok_or_else(|| "FC sandbox pool unavailable".to_string())?;
                Ok((
                    Arc::clone(fc) as Arc<dyn PoolOps + Send + Sync>,
                    FC_POOL_ID.to_string(),
                ))
            }
            WorkerExecutionBackend::PodmanPool { .. } => {
                let _ = Self::effective_mode_for_proj(db, proj_id).await;
                Ok((Arc::clone(&self.podman_pool), self.pool_id.clone()))
            }
        }
    }

    pub async fn pool_for_turn(
        &self,
        db: &GatewaySessionDb,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Result<Arc<dyn PoolOps + Send + Sync>, String> {
        if let Ok(Some(pool_id)) = db.get_turn_pool_id(turn_id, session_id, proj_id).await {
            if pool_id == FC_POOL_ID {
                if let Some(fc) = &self.fc_pool {
                    return Ok(Arc::clone(fc) as Arc<dyn PoolOps + Send + Sync>);
                }
            } else if pool_id != self.pool_id {
                tracing::warn!(
                    target: "claw_gateway_pool",
                    turn_id = %turn_id,
                    stored_pool_id = %pool_id,
                    current_pool_id = %self.pool_id,
                    "turn pool_id does not match current pool (legacy dual-pool row?)"
                );
            }
        }
        self.pool_and_id_for_proj(db, proj_id)
            .await
            .map(|(pool, _)| pool)
    }

    pub async fn interactive_backend_for_proj(
        &self,
        db: &GatewaySessionDb,
        proj_id: i64,
    ) -> Result<Arc<dyn InteractiveSandboxBackend + Send + Sync>, String> {
        if interactive_backend_is_fc() {
            return self
                .fc_interactive
                .clone()
                .ok_or_else(|| "FC interactive backend unavailable".into())
                .map(|b| b as Arc<dyn InteractiveSandboxBackend + Send + Sync>);
        }
        let json = Self::worker_json_for_proj(db, proj_id).await;
        match execution_backend_from_json(&json) {
            WorkerExecutionBackend::FcSandbox => self
                .fc_interactive
                .clone()
                .ok_or_else(|| "FC interactive backend unavailable".into())
                .map(|b| b as Arc<dyn InteractiveSandboxBackend + Send + Sync>),
            WorkerExecutionBackend::PodmanPool { .. } => {
                Ok(self.podman_interactive.clone()
                    as Arc<dyn InteractiveSandboxBackend + Send + Sync>)
            }
        }
    }

    pub async fn stop_interactive_lease(&self, lease: &InteractiveLease) -> Result<(), String> {
        match lease.backend {
            InteractiveBackendKind::Fc => {
                let fc = self
                    .fc_interactive
                    .as_ref()
                    .ok_or_else(|| "fc interactive backend missing".to_string())?;
                fc.stop_session(lease).await
            }
            InteractiveBackendKind::Podman => self.podman_interactive.stop_session(lease).await,
        }
    }

    pub async fn has_report_for_turn(&self, _db: &GatewaySessionDb, turn_id: &str) -> bool {
        if self.podman_pool.has_report_for_turn(turn_id).await {
            return true;
        }
        if let Some(fc) = &self.fc_pool {
            return fc.has_report_for_turn(turn_id).await;
        }
        false
    }

    pub async fn first_report_at_ms_for_turn(
        &self,
        _db: &GatewaySessionDb,
        turn_id: &str,
    ) -> Option<i64> {
        if let Some(ms) = self.podman_pool.first_report_at_ms_for_turn(turn_id).await {
            return Some(ms);
        }
        if let Some(fc) = &self.fc_pool {
            return fc.first_report_at_ms_for_turn(turn_id).await;
        }
        None
    }

    #[must_use]
    pub fn sandbox_rpc_client(&self) -> Option<Arc<SandboxRpcClient>> {
        self.sandbox_pool.as_ref().map(|p| p.rpc_client())
    }

    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        if let Some(pool) = self.sandbox_pool.as_ref() {
            pool.bind_session_db(Arc::clone(&db)).await;
        }
        if let Some(fc) = self.fc_pool.as_ref() {
            fc.bind_session_db(Arc::clone(&db)).await;
        }
        if let Some(fc) = &self.fc_interactive {
            fc.bind_session_db(Arc::clone(&db)).await;
        }
    }
}
