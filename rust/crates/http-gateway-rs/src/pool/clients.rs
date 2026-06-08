//! Gateway-side strict/relaxed pool RPC routing. Author: kejiqing

use std::sync::Arc;

use crate::session_db::GatewaySessionDb;

use super::config::relaxed_worker_allowed_from_env;
use super::traits::PoolOps;
use super::worker_isolation::{default_worker_isolation_json, effective_mode, WorkerIsolationMode};
use super::PoolRpcClient;

/// Strict + optional relaxed pool HTTP RPC clients for solve routing.
#[derive(Clone)]
pub struct PoolClients {
    strict: Arc<dyn PoolOps + Send + Sync>,
    relaxed: Arc<dyn PoolOps + Send + Sync>,
    strict_pool_id: String,
    relaxed_pool_id: String,
    dual_pool: bool,
}

impl PoolClients {
    /// Build from env. Dual pool when `CLAW_RELAXED_POOL_HTTP_BASE` is set.
    #[must_use]
    pub fn from_env() -> Self {
        let allow_relaxed = relaxed_worker_allowed_from_env();
        let strict_base = std::env::var("CLAW_STRICT_POOL_HTTP_BASE")
            .ok()
            .or_else(|| std::env::var("CLAW_POOL_HTTP_BASE").ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                eprintln!("http-gateway-rs: set CLAW_STRICT_POOL_HTTP_BASE or CLAW_POOL_HTTP_BASE");
                std::process::exit(1);
            });
        let relaxed_base = std::env::var("CLAW_RELAXED_POOL_HTTP_BASE")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let dual_pool = relaxed_base.is_some() && allow_relaxed;

        let strict_pool_id = std::env::var("CLAW_STRICT_POOL_ID")
            .ok()
            .or_else(|| std::env::var("CLAW_POOL_ID").ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(crate::pool_registry::resolve_pool_id);
        let relaxed_pool_id = if dual_pool {
            std::env::var("CLAW_RELAXED_POOL_ID")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| format!("{strict_pool_id}-relaxed"))
        } else {
            strict_pool_id.clone()
        };

        let strict: Arc<dyn PoolOps + Send + Sync> =
            Arc::new(PoolRpcClient::new_http(&strict_base));
        let relaxed: Arc<dyn PoolOps + Send + Sync> = if dual_pool {
            Arc::new(PoolRpcClient::new_http(
                relaxed_base
                    .as_deref()
                    .expect("dual_pool implies relaxed_base"),
            ))
        } else {
            Arc::clone(&strict)
        };

        Self {
            strict,
            relaxed,
            strict_pool_id,
            relaxed_pool_id,
            dual_pool,
        }
    }

    #[must_use]
    pub fn dual_pool(&self) -> bool {
        self.dual_pool
    }

    #[must_use]
    pub fn strict_pool_id(&self) -> &str {
        &self.strict_pool_id
    }

    #[must_use]
    pub fn relaxed_pool_id(&self) -> &str {
        &self.relaxed_pool_id
    }

    #[must_use]
    pub fn strict_client(&self) -> Arc<dyn PoolOps + Send + Sync> {
        Arc::clone(&self.strict)
    }

    /// Resolve effective isolation for a ds (global gate + project_config JSON).
    pub async fn effective_mode_for_ds(db: &GatewaySessionDb, ds_id: i64) -> WorkerIsolationMode {
        let json = db
            .get_worker_isolation_json(ds_id)
            .await
            .unwrap_or_else(|_| default_worker_isolation_json());
        effective_mode(relaxed_worker_allowed_from_env(), &json)
    }

    /// Pick pool RPC client + registry pool_id for a ds solve.
    pub async fn pool_and_id_for_ds(
        &self,
        db: &GatewaySessionDb,
        ds_id: i64,
    ) -> (Arc<dyn PoolOps + Send + Sync>, String) {
        let mode = Self::effective_mode_for_ds(db, ds_id).await;
        match mode {
            WorkerIsolationMode::Relaxed if self.dual_pool => {
                (Arc::clone(&self.relaxed), self.relaxed_pool_id.clone())
            }
            WorkerIsolationMode::Strict | WorkerIsolationMode::Relaxed => {
                (Arc::clone(&self.strict), self.strict_pool_id.clone())
            }
        }
    }

    /// Resolve pool client for an existing turn (prefer DB `pool_id`, else ds mode).
    pub async fn pool_for_turn(
        &self,
        db: &GatewaySessionDb,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Arc<dyn PoolOps + Send + Sync> {
        if let Ok(Some(pool_id)) = db.get_turn_pool_id(turn_id, session_id, ds_id).await {
            if self.dual_pool && pool_id == self.relaxed_pool_id {
                return Arc::clone(&self.relaxed);
            }
            if pool_id == self.strict_pool_id {
                return Arc::clone(&self.strict);
            }
        }
        self.pool_and_id_for_ds(db, ds_id).await.0
    }

    pub async fn has_report_for_turn(&self, _db: &GatewaySessionDb, turn_id: &str) -> bool {
        if self.strict.has_report_for_turn(turn_id).await {
            return true;
        }
        self.dual_pool && self.relaxed.has_report_for_turn(turn_id).await
    }

    pub async fn first_report_at_ms_for_turn(
        &self,
        _db: &GatewaySessionDb,
        turn_id: &str,
    ) -> Option<i64> {
        if let Some(ts) = self.strict.first_report_at_ms_for_turn(turn_id).await {
            return Some(ts);
        }
        if self.dual_pool {
            return self.relaxed.first_report_at_ms_for_turn(turn_id).await;
        }
        None
    }
}
