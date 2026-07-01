//! Per-project e2b worker registry — gateway-managed lifecycle (DB + e2b). Author: kejiqing
//!
//! One worker sandbox per `proj_id` (workspace). No interactive vs ephemeral split:
//! solve and OVS share the same proj-bound worker. Gateway startup reconciles DB rows
//! against e2b; runtime renews TTL per env; shutdown does not kill workers.
//! Template rotation is per-proj when `settings_json.e2bWorker.templateId` changes.
//! Renew TTL/interval: `CLAW_E2B_PROJECT_WORKER_TTL_SECS` (default 3600) and optional
//! `CLAW_E2B_PROJECT_WORKER_RENEW_INTERVAL_SECS` (reconcile health check; TTL touch is 60s lease ticker).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle, SANDBOX_LEASE_TICK_SECS};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::gateway_e2b_worker_settings::{
    e2b_project_worker_renew_interval_secs_from_env, e2b_project_worker_ttl_secs_from_env,
    load_e2b_worker_template_id,
};
use crate::session_db::{GatewaySessionDb, ProjectFcWorkerRow, WorkerRotationEvent};

use super::e2b_nas_layout::allocate_worker_id;
use super::NasLayoutBackend;

const PROJECT_WORKER_CONTRACT_VERSION: &str = "nas-session-root-v1";

fn worker_contract_key(template_id: &str) -> String {
    format!("{template_id}#{PROJECT_WORKER_CONTRACT_VERSION}")
}

/// Best-effort append to `worker_rotation_log`; audit never blocks worker lifecycle. Author: kejiqing
async fn audit_rotation(db: &GatewaySessionDb, event: WorkerRotationEvent) {
    if let Err(e) = db.insert_worker_rotation_event(&event).await {
        warn!(
            target: "claw_e2b_proj_worker",
            proj_id = event.proj_id,
            event = %event.event,
            error = %e,
            "worker rotation audit insert failed (best-effort)"
        );
    }
}

struct ProjWorkerRuntime {
    handle: E2bSandboxHandle,
    worker_id: String,
    #[allow(dead_code)]
    template_id: String,
}

/// In-memory cache + lease ref-count per project worker.
pub struct E2bProjWorkerRegistry {
    client: Arc<E2bSandboxClient>,
    nas_layout: NasLayoutBackend,
    db: RwLock<Option<Arc<GatewaySessionDb>>>,
    workers: Mutex<HashMap<i64, ProjWorkerRuntime>>,
    leases: Mutex<HashMap<i64, u32>>,
    worker_ttl_secs: u64,
    renew_interval_secs: u64,
}

impl E2bProjWorkerRegistry {
    #[must_use]
    pub fn new(client: Arc<E2bSandboxClient>, nas_layout: NasLayoutBackend) -> Self {
        let worker_ttl_secs = e2b_project_worker_ttl_secs_from_env();
        let renew_interval_secs = e2b_project_worker_renew_interval_secs_from_env(worker_ttl_secs);
        info!(
            target: "claw_e2b_proj_worker",
            worker_ttl_secs,
            renew_interval_secs,
            lease_tick_secs = SANDBOX_LEASE_TICK_SECS,
            "project worker renew policy from env"
        );
        Self {
            client,
            nas_layout,
            db: RwLock::new(None),
            workers: Mutex::new(HashMap::new()),
            leases: Mutex::new(HashMap::new()),
            worker_ttl_secs,
            renew_interval_secs,
        }
    }

    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        *self.db.write().await = Some(db);
    }

    async fn session_db(&self) -> Result<Arc<GatewaySessionDb>, String> {
        self.db
            .read()
            .await
            .clone()
            .ok_or_else(|| "fc proj worker registry: session db not bound".into())
    }

    /// Gateway startup: reconcile every project in DB against e2b + desired template.
    pub async fn reconcile_all_on_startup(&self) -> Result<(), String> {
        let db = self.session_db().await?;
        let template_id = load_e2b_worker_template_id(db.as_ref())
            .await
            .map_err(|e| format!("load e2bWorker template: {e}"))?;
        let desired_contract = worker_contract_key(&template_id);
        let proj_ids = db
            .list_project_config_proj_ids()
            .await
            .map_err(|e| format!("list project_config proj_ids: {e}"))?;
        info!(
            target: "claw_e2b_proj_worker",
            proj_count = proj_ids.len(),
            template_id = %template_id,
            contract = %desired_contract,
            "reconcile project e2b workers on startup"
        );
        for proj_id in proj_ids {
            if let Err(e) = self.reconcile_proj(proj_id, &template_id).await {
                warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    error = %e,
                    "reconcile proj worker failed (best-effort)"
                );
            }
        }
        self.seed_lease_tracking_from_db().await;
        Ok(())
    }

    /// Register persisted project workers for 60s TTL lease ticker.
    pub async fn seed_lease_tracking_from_db(&self) {
        let ids = self.all_persisted_sandbox_ids().await;
        if ids.is_empty() {
            return;
        }
        self.client.register_tracked_sandboxes(&ids);
        info!(
            target: "claw_e2b_proj_worker",
            count = ids.len(),
            "seeded project worker sandboxes for lease ticker"
        );
    }

    /// Per-proj: skip if online + template matches; rotate or create otherwise.
    pub async fn reconcile_proj(&self, proj_id: i64, desired_template: &str) -> Result<(), String> {
        let db = self.session_db().await?;
        let desired_contract = worker_contract_key(desired_template);
        let row = db
            .get_project_e2b_worker(proj_id)
            .await
            .map_err(|e| format!("get project_e2b_worker: {e}"))?;

        if let Some(ref existing) = row {
            if existing.template_id == desired_contract
                && self.client.sandbox_running(&existing.sandbox_id).await
            {
                let handle = E2bSandboxClient::handle_from_json(&existing.handle_json)?;
                self.cache_worker(
                    proj_id,
                    handle,
                    existing.worker_id.clone(),
                    existing.template_id.clone(),
                )
                .await;
                self.client
                    .renew_sandbox_ttl_secs(&existing.sandbox_id, self.worker_ttl_secs)
                    .await
                    .map_err(|e| format!("renew existing project worker TTL: {e}"))?;
                info!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    sandbox_id = %existing.sandbox_id,
                    ttl_secs = self.worker_ttl_secs,
                    contract = %desired_contract,
                    "proj worker online — skip create"
                );
                return Ok(());
            }
            info!(
                target: "claw_e2b_proj_worker",
                proj_id,
                old_sandbox = %existing.sandbox_id,
                old_template = %existing.template_id,
                new_template = %desired_contract,
                "proj worker rotate (template mismatch or offline)"
            );
            if self.client.sandbox_running(&existing.sandbox_id).await {
                let _ = self.client.kill_sandbox(&existing.sandbox_id).await;
            }
            audit_rotation(
                db.as_ref(),
                WorkerRotationEvent {
                    proj_id,
                    event: "rotated_out".to_string(),
                    sandbox_id: Some(existing.sandbox_id.clone()),
                    worker_id: Some(existing.worker_id.clone()),
                    template_id: Some(existing.template_id.clone()),
                    reason: Some("template_mismatch_or_offline".to_string()),
                    at_ms: chrono::Utc::now().timestamp_millis(),
                },
            )
            .await;
            let _ = db.delete_project_e2b_worker(proj_id).await;
            self.workers.lock().await.remove(&proj_id);
        }

        self.create_and_persist(proj_id, desired_template).await
    }

    async fn create_and_persist(&self, proj_id: i64, template_id: &str) -> Result<(), String> {
        let db = self.session_db().await?;
        let contract_key = worker_contract_key(template_id);
        let worker_id = allocate_worker_id();
        self.nas_layout
            .prepare_e2b_worker_bind_sources(db.as_ref(), proj_id, &worker_id)
            .await?;
        let handle = self
            .client
            .create_warm_proj_sandbox(&self.nas_layout.cluster_id()?, proj_id, &worker_id)
            .await?;
        if let Err(e) = self
            .client
            .renew_sandbox_ttl_secs(&handle.sandbox_id, self.worker_ttl_secs)
            .await
        {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!("renew new project worker TTL: {e}"));
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        let row = ProjectFcWorkerRow {
            proj_id,
            sandbox_id: handle.sandbox_id.clone(),
            worker_id: worker_id.clone(),
            template_id: contract_key.clone(),
            handle_json: E2bSandboxClient::handle_to_json(&handle),
            updated_at_ms: now_ms,
        };
        db.upsert_project_e2b_worker(&row)
            .await
            .map_err(|e| format!("upsert project_e2b_worker: {e}"))?;
        audit_rotation(
            db.as_ref(),
            WorkerRotationEvent {
                proj_id,
                event: "created".to_string(),
                sandbox_id: Some(row.sandbox_id.clone()),
                worker_id: Some(row.worker_id.clone()),
                template_id: Some(contract_key.clone()),
                reason: None,
                at_ms: now_ms,
            },
        )
        .await;

        self.cache_worker(proj_id, handle.clone(), worker_id, contract_key.clone())
            .await;
        info!(
            target: "claw_e2b_proj_worker",
            proj_id,
            sandbox_id = %handle.sandbox_id,
            template_id,
            contract = %contract_key,
            ttl_secs = self.worker_ttl_secs,
            "proj worker created and persisted"
        );
        Ok(())
    }

    async fn cache_worker(
        &self,
        proj_id: i64,
        handle: E2bSandboxHandle,
        worker_id: String,
        template_id: String,
    ) {
        self.client.register_tracked_sandbox(&handle.sandbox_id);
        self.workers.lock().await.insert(
            proj_id,
            ProjWorkerRuntime {
                handle,
                worker_id,
                template_id,
            },
        );
    }

    /// Ensure proj worker exists (reconcile on demand) and return handle + worker_id.
    pub async fn ensure_worker(&self, proj_id: i64) -> Result<(E2bSandboxHandle, String), String> {
        {
            let guard = self.workers.lock().await;
            if let Some(rt) = guard.get(&proj_id) {
                if self.client.sandbox_running(&rt.handle.sandbox_id).await {
                    return Ok((rt.handle.clone(), rt.worker_id.clone()));
                }
            }
        }
        let db = self.session_db().await?;
        let template_id = load_e2b_worker_template_id(db.as_ref())
            .await
            .map_err(|e| format!("load e2bWorker template: {e}"))?;
        self.reconcile_proj(proj_id, &template_id).await?;
        let guard = self.workers.lock().await;
        let rt = guard
            .get(&proj_id)
            .ok_or_else(|| format!("proj worker missing after reconcile proj_{proj_id}"))?;
        Ok((rt.handle.clone(), rt.worker_id.clone()))
    }

    /// Lease proj worker for a turn or interactive session (ref-count).
    pub async fn acquire(&self, proj_id: i64) -> Result<(E2bSandboxHandle, String), String> {
        let (handle, worker_id) = self.ensure_worker(proj_id).await?;
        self.client
            .renew_sandbox_ttl_secs(&handle.sandbox_id, self.worker_ttl_secs)
            .await
            .map_err(|e| format!("renew acquired project worker TTL: {e}"))?;
        let mut leases = self.leases.lock().await;
        *leases.entry(proj_id).or_insert(0) += 1;
        Ok((handle, worker_id))
    }

    /// Release lease — worker stays alive (no kill).
    pub async fn release(&self, proj_id: i64) {
        let mut leases = self.leases.lock().await;
        if let Some(n) = leases.get_mut(&proj_id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                leases.remove(&proj_id);
            }
        }
    }

    #[must_use]
    pub async fn leased_handle(&self, proj_id: i64) -> Option<E2bSandboxHandle> {
        self.workers
            .lock()
            .await
            .get(&proj_id)
            .map(|rt| rt.handle.clone())
    }

    pub async fn all_persisted_sandbox_ids(&self) -> Vec<String> {
        if let Some(db) = self.db.read().await.clone() {
            if let Ok(ids) = db.list_project_e2b_worker_sandbox_ids().await {
                return ids;
            }
        }
        self.workers
            .lock()
            .await
            .values()
            .map(|rt| rt.handle.sandbox_id.clone())
            .collect()
    }

    /// Background reconcile health check (TTL touch is [`SANDBOX_LEASE_TICK_SECS`] lease ticker).
    pub fn spawn_renewal_ticker(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(self.renew_interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Ok(db) = self.session_db().await else {
                    continue;
                };
                let template_id = match load_e2b_worker_template_id(db.as_ref()).await {
                    Ok(t) => t,
                    Err(e) => {
                        warn!(target: "claw_e2b_proj_worker", error = %e, "renewal ticker: template load failed");
                        continue;
                    }
                };
                let proj_ids = match db.list_project_config_proj_ids().await {
                    Ok(ids) => ids,
                    Err(e) => {
                        warn!(target: "claw_e2b_proj_worker", error = %e, "renewal ticker: list proj_ids failed");
                        continue;
                    }
                };
                for proj_id in proj_ids {
                    if let Err(e) = self.reconcile_proj(proj_id, &template_id).await {
                        warn!(
                            target: "claw_e2b_proj_worker",
                            proj_id,
                            error = %e,
                            "renewal ticker reconcile failed"
                        );
                    }
                }
            }
        });
    }

    /// Force kill + recreate project worker on latest template (admin reset). Author: kejiqing
    pub async fn force_rotate_proj(&self, proj_id: i64) -> Result<(), String> {
        let db = self.session_db().await?;
        let template_id = load_e2b_worker_template_id(db.as_ref())
            .await
            .map_err(|e| format!("load e2bWorker template: {e}"))?;
        let row = db
            .get_project_e2b_worker(proj_id)
            .await
            .map_err(|e| format!("get project_e2b_worker: {e}"))?;
        if let Some(ref existing) = row {
            info!(
                target: "claw_e2b_proj_worker",
                proj_id,
                sandbox_id = %existing.sandbox_id,
                "admin force rotate project worker"
            );
            if self.client.sandbox_running(&existing.sandbox_id).await {
                let _ = self.client.kill_sandbox(&existing.sandbox_id).await;
            }
            audit_rotation(
                db.as_ref(),
                WorkerRotationEvent {
                    proj_id,
                    event: "rotated_out".to_string(),
                    sandbox_id: Some(existing.sandbox_id.clone()),
                    worker_id: Some(existing.worker_id.clone()),
                    template_id: Some(existing.template_id.clone()),
                    reason: Some("admin_force_reset".to_string()),
                    at_ms: chrono::Utc::now().timestamp_millis(),
                },
            )
            .await;
            let _ = db.delete_project_e2b_worker(proj_id).await;
            self.workers.lock().await.remove(&proj_id);
        }
        self.create_and_persist(proj_id, &template_id).await
    }

    /// Gateway shutdown: workers survive (no kill).
    pub async fn shutdown_all(&self) {
        self.workers.lock().await.clear();
        self.leases.lock().await.clear();
        info!(target: "claw_e2b_proj_worker", "shutdown_all (workers left running on e2b)");
    }
}
