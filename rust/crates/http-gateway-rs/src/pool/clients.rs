//! Gateway → e2b cloud sandbox (solve + interactive). Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;

use claw_e2b_sandbox_client::E2bSandboxClient;

use crate::session_db::GatewaySessionDb;

use super::config::relaxed_worker_allowed_from_env;
use super::e2b_orchestrated_pool::{E2bOrchestratedPool, E2B_POOL_ID};
use super::e2b_proj_worker_registry::E2bProjWorkerRegistry;
use super::interactive_backend::{
    E2bInteractiveBackend, InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend,
};
use super::traits::PoolOps;
use super::worker_profile::{
    default_worker_profile_json, effective_mode, mode_from_json, WorkerProfileMode,
};
use super::{LiveReportHub, NasLayoutBackend};

/// e2b-only pool routing. Author: kejiqing
#[derive(Clone)]
pub struct PoolClients {
    e2b_pool: Arc<E2bOrchestratedPool>,
    e2b_workers: Arc<E2bProjWorkerRegistry>,
    pool_id: String,
    e2b_interactive: Arc<E2bInteractiveBackend>,
    e2b_client: Arc<E2bSandboxClient>,
    work_root: PathBuf,
    pool_rpc_host_work_root: Option<PathBuf>,
    nas_layout: NasLayoutBackend,
}

impl PoolClients {
    #[must_use]
    pub fn from_env(
        live_report_hub: Arc<LiveReportHub>,
        work_root: PathBuf,
        e2b_client: Option<Arc<E2bSandboxClient>>,
        pool_rpc_host_work_root: Option<PathBuf>,
        nas_layout: NasLayoutBackend,
    ) -> Self {
        let pool_id = std::env::var("CLAW_POOL_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(crate::pool_registry::resolve_pool_id);

        let e2b_client = e2b_client.unwrap_or_else(|| {
            eprintln!(
                "http-gateway-rs: e2b sandbox is required; configure CLAW_E2B_* / CLAW_E2B_*"
            );
            std::process::exit(1);
        });
        if let Err(e) = nas_layout.cluster_id() {
            eprintln!("http-gateway-rs: CLAW_CLUSTER_ID is required for e2b NAS layout: {e}");
            std::process::exit(1);
        }
        if !nas_layout.uses_nas_api() {
            eprintln!(
                "http-gateway-rs: e2b requires claw-nas-api; deploy: ./deploy/stack/gateway.sh nas-api-up"
            );
            std::process::exit(1);
        }

        let e2b_workers = Arc::new(E2bProjWorkerRegistry::new(
            Arc::clone(&e2b_client),
            nas_layout.clone(),
        ));
        E2bSandboxClient::spawn_lease_ticker(Arc::clone(&e2b_client));
        E2bProjWorkerRegistry::spawn_renewal_ticker(Arc::clone(&e2b_workers));

        let e2b_pool = Arc::new(E2bOrchestratedPool::new(
            Arc::clone(&e2b_client),
            live_report_hub,
            nas_layout.clone(),
            Arc::clone(&e2b_workers),
        ));
        let e2b_interactive = Arc::new(E2bInteractiveBackend::new(
            Arc::clone(&e2b_client),
            pool_id.clone(),
            nas_layout.clone(),
            Arc::clone(&e2b_workers),
        ));

        Self {
            e2b_pool,
            e2b_workers,
            pool_id,
            e2b_interactive,
            e2b_client,
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
        super::e2b_nas_layout::nas_host_root(
            &self.work_root,
            self.pool_rpc_host_work_root.as_deref(),
        )
    }

    #[must_use]
    pub fn e2b_nas_layout_active(&self) -> bool {
        self.nas_layout.active()
    }

    #[must_use]
    pub fn e2b_interactive(&self) -> Option<&E2bInteractiveBackend> {
        Some(&self.e2b_interactive)
    }

    #[must_use]
    pub fn e2b_interactive_arc(&self) -> Arc<dyn InteractiveSandboxBackend + Send + Sync> {
        Arc::clone(&self.e2b_interactive) as Arc<dyn InteractiveSandboxBackend + Send + Sync>
    }

    #[must_use]
    pub fn e2b_worker_registry(&self) -> &E2bProjWorkerRegistry {
        &self.e2b_workers
    }

    #[must_use]
    pub fn e2b_sandbox_client(&self) -> Option<&Arc<E2bSandboxClient>> {
        Some(&self.e2b_client)
    }

    /// Graceful shutdown: project workers + singletons survive on e2b; kill only ephemeral leases.
    pub async fn shutdown_e2b_sandboxes(&self) {
        let cluster_id = std::env::var("CLAW_CLUSTER_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "default".to_string());

        self.e2b_interactive.shutdown_all().await;

        let mut skip = self.e2b_workers.all_persisted_sandbox_ids().await;
        skip.extend(self.e2b_client.persistent_sandbox_ids());
        skip.sort();
        skip.dedup();
        let leased = self
            .e2b_client
            .kill_all_leased_sandboxes_except(&skip)
            .await;
        let orphans = self
            .e2b_client
            .kill_cluster_singleton_orphans(&cluster_id)
            .await
            .unwrap_or(0);
        tracing::info!(
            target: "claw_e2b_sandbox",
            cluster_id = %cluster_id,
            persisted_workers = skip.len(),
            ephemeral_killed = leased,
            orphan_singletons_killed = orphans,
            "shutdown_e2b_sandboxes complete (project workers left running)"
        );
    }

    #[must_use]
    pub fn pool_id(&self) -> &str {
        &self.pool_id
    }

    #[must_use]
    pub fn client(&self) -> Arc<dyn PoolOps + Send + Sync> {
        Arc::clone(&self.e2b_pool) as Arc<dyn PoolOps + Send + Sync>
    }

    pub async fn worker_json_for_proj(db: &GatewaySessionDb, proj_id: i64) -> serde_json::Value {
        db.get_worker_profile_json(proj_id)
            .await
            .unwrap_or_else(|_| default_worker_profile_json())
    }

    pub async fn effective_mode_for_proj(db: &GatewaySessionDb, proj_id: i64) -> WorkerProfileMode {
        let json = Self::worker_json_for_proj(db, proj_id).await;
        effective_mode(relaxed_worker_allowed_from_env(), &json)
    }

    pub async fn assert_proj_worker_profile_supported(
        &self,
        db: &GatewaySessionDb,
        proj_id: i64,
    ) -> Result<(), String> {
        let json = Self::worker_json_for_proj(db, proj_id).await;
        if mode_from_json(&json) != WorkerProfileMode::Relaxed {
            return Ok(());
        }
        if !relaxed_worker_allowed_from_env() {
            return Err(
                "proj worker_profile_json.mode=relaxed but CLAW_ALLOW_RELAXED_WORKER=false on gateway; \
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
        self.assert_proj_worker_profile_supported(db, proj_id)
            .await?;
        Ok((
            Arc::clone(&self.e2b_pool) as Arc<dyn PoolOps + Send + Sync>,
            E2B_POOL_ID.to_string(),
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
            if pool_id == E2B_POOL_ID {
                return Ok(Arc::clone(&self.e2b_pool) as Arc<dyn PoolOps + Send + Sync>);
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

    #[allow(clippy::unused_async)]
    pub async fn interactive_backend_for_proj(
        &self,
        _db: &GatewaySessionDb,
        _proj_id: i64,
    ) -> Result<Arc<dyn InteractiveSandboxBackend + Send + Sync>, String> {
        Ok(self.e2b_interactive.clone() as Arc<dyn InteractiveSandboxBackend + Send + Sync>)
    }

    pub async fn stop_interactive_lease(&self, lease: &InteractiveLease) -> Result<(), String> {
        match lease.backend {
            InteractiveBackendKind::E2b => self.e2b_interactive.stop_session(lease).await,
        }
    }

    pub async fn has_report_for_turn(&self, _db: &GatewaySessionDb, turn_id: &str) -> bool {
        self.e2b_pool.has_report_for_turn(turn_id).await
    }

    pub async fn first_report_at_ms_for_turn(
        &self,
        _db: &GatewaySessionDb,
        turn_id: &str,
    ) -> Option<i64> {
        self.e2b_pool.first_report_at_ms_for_turn(turn_id).await
    }

    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        self.e2b_pool.bind_session_db(Arc::clone(&db)).await;
        self.e2b_workers.bind_session_db(Arc::clone(&db)).await;
        self.e2b_interactive.bind_session_db(db).await;
    }

    /// Startup reconcile: ensure every project's worker exists on e2b and matches template.
    pub async fn reconcile_project_workers_on_startup(&self) -> Result<(), String> {
        self.e2b_workers.reconcile_all_on_startup().await
    }

    /// Reconcile all strict project worker pools (e.g. after Admin poolSize change).
    pub async fn reconcile_all_project_workers(&self) -> Result<(), String> {
        self.e2b_workers.reconcile_all_projects().await
    }

    /// Reconcile one project's workers (e.g. after per-project worker_profile poolSize change).
    pub async fn reconcile_project_worker(&self, proj_id: i64) -> Result<(), String> {
        self.e2b_workers.reconcile_proj(proj_id).await
    }

    /// Startup gate: nas-api + observe must be online before gateway serves traffic.
    pub async fn ensure_e2b_singletons_on_startup_strict(
        &self,
        db: &GatewaySessionDb,
    ) -> Result<(), String> {
        crate::gateway_e2b_singleton_lifecycle::ensure_e2b_singletons_on_startup_strict(
            db,
            self.e2b_client.as_ref(),
        )
        .await
    }

    pub fn spawn_singleton_health_reconcile_loop(&self, db: Arc<GatewaySessionDb>) {
        crate::gateway_e2b_singleton_lifecycle::spawn_singleton_health_reconcile_loop(
            db,
            Arc::clone(&self.e2b_client),
        );
    }
}
