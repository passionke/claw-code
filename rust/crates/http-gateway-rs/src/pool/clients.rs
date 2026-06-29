//! Gateway → FC cloud sandbox (solve + interactive). Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;

use claw_fc_sandbox_client::FcSandboxClient;

use crate::session_db::GatewaySessionDb;

use super::config::relaxed_worker_allowed_from_env;
use super::fc_orchestrated_pool::{FcOrchestratedPool, FC_POOL_ID};
use super::fc_proj_worker_registry::FcProjWorkerRegistry;
use super::interactive_backend::{
    FcInteractiveBackend, InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend,
};
use super::traits::PoolOps;
use super::worker_isolation::{
    default_worker_isolation_json, effective_mode, is_fc_sandbox_mode, mode_from_json,
    WorkerIsolationMode,
};
use super::{LiveReportHub, NasLayoutBackend};

/// FC-only pool routing. Author: kejiqing
#[derive(Clone)]
pub struct PoolClients {
    fc_pool: Arc<FcOrchestratedPool>,
    fc_workers: Arc<FcProjWorkerRegistry>,
    pool_id: String,
    fc_interactive: Arc<FcInteractiveBackend>,
    fc_client: Arc<FcSandboxClient>,
    work_root: PathBuf,
    pool_rpc_host_work_root: Option<PathBuf>,
    nas_layout: NasLayoutBackend,
}

impl PoolClients {
    #[must_use]
    pub fn from_env(
        live_report_hub: Arc<LiveReportHub>,
        work_root: PathBuf,
        fc_client: Option<Arc<FcSandboxClient>>,
        pool_rpc_host_work_root: Option<PathBuf>,
        nas_layout: NasLayoutBackend,
    ) -> Self {
        let pool_id = std::env::var("CLAW_POOL_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(crate::pool_registry::resolve_pool_id);

        let fc_client = fc_client.unwrap_or_else(|| {
            eprintln!(
                "http-gateway-rs: FC/E2B sandbox is required; configure CLAW_FC_* / CLAW_E2B_*"
            );
            std::process::exit(1);
        });
        if let Err(e) = nas_layout.cluster_id() {
            eprintln!("http-gateway-rs: CLAW_CLUSTER_ID is required for FC/E2B NAS layout: {e}");
            std::process::exit(1);
        }
        if !nas_layout.active() {
            eprintln!(
                "http-gateway-rs: FC/E2B NAS layout is required; configure claw-nas-api or CLAW_NAS_HOST_MOUNT/CLAW_WORK_ROOT"
            );
            std::process::exit(1);
        }

        let fc_workers = Arc::new(FcProjWorkerRegistry::new(
            Arc::clone(&fc_client),
            nas_layout.clone(),
        ));
        FcProjWorkerRegistry::spawn_renewal_ticker(Arc::clone(&fc_workers));

        let fc_pool = Arc::new(FcOrchestratedPool::new(
            Arc::clone(&fc_client),
            work_root.clone(),
            live_report_hub,
            nas_layout.clone(),
            Arc::clone(&fc_workers),
        ));
        let fc_interactive = Arc::new(FcInteractiveBackend::new(
            Arc::clone(&fc_client),
            pool_id.clone(),
            nas_layout.clone(),
            Arc::clone(&fc_workers),
        ));

        Self {
            fc_pool,
            fc_workers,
            pool_id,
            fc_interactive,
            fc_client,
            work_root,
            pool_rpc_host_work_root,
            nas_layout,
        }
    }

    #[must_use]
    pub fn nas_layout(&self) -> &NasLayoutBackend {
        &self.nas_layout
    }

    #[must_use]
    pub fn nas_host_root(&self) -> PathBuf {
        super::fc_nas_layout::nas_host_root(
            &self.work_root,
            self.pool_rpc_host_work_root.as_deref(),
        )
    }

    #[must_use]
    pub fn fc_nas_layout_active(&self) -> bool {
        self.nas_layout.active()
    }

    #[must_use]
    pub fn fc_interactive(&self) -> Option<&FcInteractiveBackend> {
        Some(&self.fc_interactive)
    }

    #[must_use]
    pub fn fc_interactive_arc(&self) -> Arc<dyn InteractiveSandboxBackend + Send + Sync> {
        Arc::clone(&self.fc_interactive) as Arc<dyn InteractiveSandboxBackend + Send + Sync>
    }

    #[must_use]
    pub fn fc_worker_registry(&self) -> &FcProjWorkerRegistry {
        &self.fc_workers
    }

    #[must_use]
    pub fn fc_sandbox_client(&self) -> Option<&Arc<FcSandboxClient>> {
        Some(&self.fc_client)
    }

    /// Graceful shutdown: project workers survive on e2b; kill only non-persisted leases.
    pub async fn shutdown_fc_sandboxes(&self) {
        let cluster_id = std::env::var("CLAW_CLUSTER_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "default".to_string());

        self.fc_interactive.shutdown_all().await;

        let skip = self.fc_workers.all_persisted_sandbox_ids().await;
        let leased = self.fc_client.kill_all_leased_sandboxes_except(&skip).await;
        let orphans = self
            .fc_client
            .kill_cluster_singleton_orphans(&cluster_id)
            .await
            .unwrap_or(0);
        tracing::info!(
            target: "claw_fc_sandbox",
            cluster_id = %cluster_id,
            persisted_workers = skip.len(),
            ephemeral_killed = leased,
            orphan_singletons_killed = orphans,
            "shutdown_fc_sandboxes complete (project workers left running)"
        );
    }

    #[must_use]
    pub fn pool_id(&self) -> &str {
        &self.pool_id
    }

    #[must_use]
    pub fn client(&self) -> Arc<dyn PoolOps + Send + Sync> {
        Arc::clone(&self.fc_pool) as Arc<dyn PoolOps + Send + Sync>
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
            return Ok(());
        }
        if mode_from_json(&json) != WorkerIsolationMode::Relaxed {
            return Ok(());
        }
        if !relaxed_worker_allowed_from_env() {
            return Err(
                "proj worker_isolation_json.mode=relaxed but CLAW_ALLOW_RELAXED_WORKER=false on gateway; \
                 set CLAW_ALLOW_RELAXED_WORKER=true in repo .env and restart gateway, or set proj to strict"
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
        let _ = db;
        let _ = proj_id;
        self.assert_proj_worker_isolation_supported(db, proj_id)
            .await?;
        Ok((
            Arc::clone(&self.fc_pool) as Arc<dyn PoolOps + Send + Sync>,
            FC_POOL_ID.to_string(),
        ))
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
                return Ok(Arc::clone(&self.fc_pool) as Arc<dyn PoolOps + Send + Sync>);
            }
            if pool_id != self.pool_id {
                tracing::warn!(
                    target: "claw_gateway_pool",
                    turn_id = %turn_id,
                    stored_pool_id = %pool_id,
                    current_pool_id = %self.pool_id,
                    "turn pool_id does not match current pool (legacy row?)"
                );
            }
        }
        self.pool_and_id_for_proj(db, proj_id)
            .await
            .map(|(pool, _)| pool)
    }

    pub async fn interactive_backend_for_proj(
        &self,
        _db: &GatewaySessionDb,
        _proj_id: i64,
    ) -> Result<Arc<dyn InteractiveSandboxBackend + Send + Sync>, String> {
        Ok(self.fc_interactive.clone() as Arc<dyn InteractiveSandboxBackend + Send + Sync>)
    }

    pub async fn stop_interactive_lease(&self, lease: &InteractiveLease) -> Result<(), String> {
        match lease.backend {
            InteractiveBackendKind::Fc => self.fc_interactive.stop_session(lease).await,
        }
    }

    pub async fn has_report_for_turn(&self, _db: &GatewaySessionDb, turn_id: &str) -> bool {
        self.fc_pool.has_report_for_turn(turn_id).await
    }

    pub async fn first_report_at_ms_for_turn(
        &self,
        _db: &GatewaySessionDb,
        turn_id: &str,
    ) -> Option<i64> {
        self.fc_pool.first_report_at_ms_for_turn(turn_id).await
    }

    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        self.fc_pool.bind_session_db(Arc::clone(&db)).await;
        self.fc_workers.bind_session_db(Arc::clone(&db)).await;
        self.fc_interactive.bind_session_db(db).await;
    }

    /// Startup reconcile: ensure every project's worker exists on e2b and matches template.
    pub async fn reconcile_project_workers_on_startup(&self) -> Result<(), String> {
        self.fc_workers.reconcile_all_on_startup().await
    }
}
