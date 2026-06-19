//! Gateway → claw-sandbox RPC client. Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;

use claw_sandbox_client::SandboxRpcClient;

use crate::session_db::GatewaySessionDb;

use super::config::relaxed_worker_allowed_from_env;
use super::sandbox_orchestrator::SandboxOrchestratedPool;
use super::traits::PoolOps;
use super::worker_isolation::{
    default_worker_isolation_json, effective_mode, mode_from_json, WorkerIsolationMode,
};
use super::LiveReportHub;

/// Single pool HTTP client (strict + relaxed workers share one claw-sandbox). Author: kejiqing
#[derive(Clone)]
pub struct PoolClients {
    pool: Arc<dyn PoolOps + Send + Sync>,
    pool_id: String,
    sandbox_pool: Option<Arc<SandboxOrchestratedPool>>,
}

impl PoolClients {
    #[must_use]
    pub fn from_env(live_report_hub: Arc<LiveReportHub>, work_root: PathBuf) -> Self {
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
        let orch =
            SandboxOrchestratedPool::new(client, work_root, pool_id.clone(), live_report_hub);
        let sandbox_pool = Some(Arc::clone(&orch));
        let pool: Arc<dyn PoolOps + Send + Sync> = orch;

        Self {
            pool,
            pool_id,
            sandbox_pool,
        }
    }

    #[must_use]
    pub fn pool_id(&self) -> &str {
        &self.pool_id
    }

    #[must_use]
    pub fn client(&self) -> Arc<dyn PoolOps + Send + Sync> {
        Arc::clone(&self.pool)
    }

    pub async fn effective_mode_for_proj(
        db: &GatewaySessionDb,
        proj_id: i64,
    ) -> WorkerIsolationMode {
        let json = db
            .get_worker_isolation_json(proj_id)
            .await
            .unwrap_or_else(|_| default_worker_isolation_json());
        effective_mode(relaxed_worker_allowed_from_env(), &json)
    }

    pub async fn assert_proj_worker_isolation_supported(
        &self,
        db: &GatewaySessionDb,
        proj_id: i64,
    ) -> Result<(), String> {
        let json = db
            .get_worker_isolation_json(proj_id)
            .await
            .unwrap_or_else(|_| default_worker_isolation_json());
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
    ) -> (Arc<dyn PoolOps + Send + Sync>, String) {
        let _ = Self::effective_mode_for_proj(db, proj_id).await;
        (Arc::clone(&self.pool), self.pool_id.clone())
    }

    pub async fn pool_for_turn(
        &self,
        db: &GatewaySessionDb,
        turn_id: &str,
        session_id: &str,
        proj_id: i64,
    ) -> Arc<dyn PoolOps + Send + Sync> {
        if let Ok(Some(pool_id)) = db.get_turn_pool_id(turn_id, session_id, proj_id).await {
            if pool_id != self.pool_id {
                tracing::warn!(
                    target: "claw_gateway_pool",
                    turn_id = %turn_id,
                    stored_pool_id = %pool_id,
                    current_pool_id = %self.pool_id,
                    "turn pool_id does not match current pool (legacy dual-pool row?)"
                );
            }
        }
        Arc::clone(&self.pool)
    }

    pub async fn has_report_for_turn(&self, _db: &GatewaySessionDb, turn_id: &str) -> bool {
        self.pool.has_report_for_turn(turn_id).await
    }

    pub async fn first_report_at_ms_for_turn(
        &self,
        _db: &GatewaySessionDb,
        turn_id: &str,
    ) -> Option<i64> {
        self.pool.first_report_at_ms_for_turn(turn_id).await
    }

    #[must_use]
    pub fn sandbox_rpc_client(&self) -> Option<Arc<SandboxRpcClient>> {
        self.sandbox_pool.as_ref().map(|p| p.rpc_client())
    }

    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        if let Some(pool) = self.sandbox_pool.as_ref() {
            pool.bind_session_db(Arc::clone(&db)).await;
        }
    }
}
