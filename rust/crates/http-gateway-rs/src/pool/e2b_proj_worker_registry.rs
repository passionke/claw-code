//! Per-project e2b worker registry — gateway-managed lifecycle (DB + e2b). Author: kejiqing
//!
//! One worker sandbox per `proj_id` (workspace). Relaxed projects use `claw-worker-relaxed`
//! (built-in OVS on :3000); strict projects use PG `e2bWorker.templateId`. Gateway startup
//! reconciles DB rows against e2b; runtime renews TTL per env; shutdown does not kill workers.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle, SANDBOX_LEASE_TICK_SECS};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::gateway_e2b_worker_settings::{
    e2b_project_worker_renew_interval_secs_from_env, e2b_project_worker_ttl_secs_from_env,
    load_e2b_worker_relaxed_template_id, load_e2b_worker_template_id,
};
use crate::project_config_draft;
use crate::session_db::{GatewaySessionDb, ProjectFcWorkerRow, WorkerRotationEvent};

use super::config::relaxed_worker_allowed_from_env;
use super::e2b_nas_layout::allocate_worker_id;
use super::worker_profile::{
    default_worker_profile_json, effective_mode, profile_mode_label, WorkerProfileMode,
};
use super::NasLayoutBackend;

const PROJECT_WORKER_CONTRACT_VERSION: &str = "nas-session-root-v3";
/// e2b alias for relaxed worker; PG may store `tpl_*` for the same template.
const RELAXED_WORKER_ALIAS: &str = "claw-worker-relaxed";

fn worker_contract_key(template_id: &str, project_home_rev: &str, profile: &str) -> String {
    format!(
        "{template_id}#{PROJECT_WORKER_CONTRACT_VERSION}#home={project_home_rev}#profile={profile}"
    )
}

async fn desired_worker_contract(
    db: &GatewaySessionDb,
    template_id: &str,
    proj_id: i64,
    profile: &str,
) -> Result<String, String> {
    let home_rev = match project_config_draft::row_for_materialize(db, proj_id).await {
        Ok(Some(row)) => row.content_rev,
        Ok(None) => "none".to_string(),
        Err(e) => return Err(format!("load project home rev for worker contract: {e}")),
    };
    Ok(worker_contract_key(template_id, &home_rev, profile))
}

#[derive(Debug, PartialEq, Eq)]
struct WorkerContractParts {
    template: String,
    version: String,
    home_rev: String,
    profile: String,
}

fn parse_worker_contract(key: &str) -> Option<WorkerContractParts> {
    let mut parts = key.splitn(4, '#');
    let template = parts.next()?.to_string();
    let version = parts.next()?.to_string();
    let home = parts.next()?.strip_prefix("home=")?.to_string();
    let profile = parts.next()?.strip_prefix("profile=")?.to_string();
    Some(WorkerContractParts {
        template,
        version,
        home_rev: home,
        profile,
    })
}

/// Whether template change alone should trigger sandbox rotation.
fn template_rotation_needed(stored_tpl: &str, desired_tpl: &str) -> bool {
    if stored_tpl == desired_tpl {
        return false;
    }
    let stored_is_alias = stored_tpl == RELAXED_WORKER_ALIAS;
    let desired_is_alias = desired_tpl == RELAXED_WORKER_ALIAS;
    if stored_is_alias || desired_is_alias {
        // Relaxed alias vs PG `tpl_*` is the same lineage — relabel contract, do not rotate.
        return false;
    }
    if stored_tpl.starts_with("tpl_") && desired_tpl.starts_with("tpl_") {
        return stored_tpl != desired_tpl;
    }
    true
}

/// True when home/profile/version differ, or template change requires a new sandbox.
fn contract_requires_rotation(stored: &str, desired: &str) -> bool {
    if stored == desired {
        return false;
    }
    let Some(stored_parts) = parse_worker_contract(stored) else {
        return true;
    };
    let Some(desired_parts) = parse_worker_contract(desired) else {
        return true;
    };
    if stored_parts.version != desired_parts.version
        || stored_parts.home_rev != desired_parts.home_rev
        || stored_parts.profile != desired_parts.profile
    {
        return true;
    }
    template_rotation_needed(&stored_parts.template, &desired_parts.template)
}

struct WorkerSpec {
    e2b_template_id: String,
    include_ovs: bool,
    profile_label: String,
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

    async fn desired_worker_spec(&self, proj_id: i64) -> Result<WorkerSpec, String> {
        let db = self.session_db().await?;
        let json = db
            .get_worker_profile_json(proj_id)
            .await
            .unwrap_or_else(|_| default_worker_profile_json());
        let mode = effective_mode(relaxed_worker_allowed_from_env(), &json);
        let profile_label = profile_mode_label(&json).to_string();
        match mode {
            WorkerProfileMode::Relaxed => {
                let e2b_template_id = load_e2b_worker_relaxed_template_id(db.as_ref())
                    .await
                    .map_err(|e| format!("load e2bWorkerRelaxed template: {e}"))?;
                Ok(WorkerSpec {
                    e2b_template_id,
                    include_ovs: true,
                    profile_label,
                })
            }
            WorkerProfileMode::Strict => {
                let e2b_template_id = load_e2b_worker_template_id(db.as_ref())
                    .await
                    .map_err(|e| format!("load e2bWorker template: {e}"))?;
                Ok(WorkerSpec {
                    e2b_template_id,
                    include_ovs: false,
                    profile_label,
                })
            }
        }
    }

    async fn relaxed_ovs_http_ok(&self, handle: &E2bSandboxHandle) -> bool {
        let Some(base) = handle.ovs_base_url.as_deref().filter(|u| !u.is_empty()) else {
            return false;
        };
        let url = format!("{}/", base.trim_end_matches('/'));
        let Ok(client) = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
        else {
            return false;
        };
        match client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    fn normalize_handle(&self, mut handle: E2bSandboxHandle, include_ovs: bool) -> E2bSandboxHandle {
        if include_ovs {
            handle = E2bSandboxClient::handle_with_builtin_ovs(handle, &self.client);
        }
        handle
    }

    /// Gateway startup: reconcile every project in DB against e2b + desired template.
    pub async fn reconcile_all_on_startup(&self) -> Result<(), String> {
        let db = self.session_db().await?;
        let proj_ids = db
            .list_project_config_proj_ids()
            .await
            .map_err(|e| format!("list project_config proj_ids: {e}"))?;
        info!(
            target: "claw_e2b_proj_worker",
            proj_count = proj_ids.len(),
            "reconcile project e2b workers on startup"
        );
        for proj_id in proj_ids {
            if let Err(e) = self.reconcile_proj(proj_id).await {
                warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    error = %e,
                    "reconcile proj worker failed (best-effort)"
                );
            }
        }
        self.reap_cluster_warm_proj_orphans_best_effort().await;
        self.seed_lease_tracking_from_db().await;
        Ok(())
    }

    /// Kill stray warm-proj sandboxes not registered in PG (rotation leftovers).
    async fn reap_cluster_warm_proj_orphans_best_effort(&self) {
        let Ok(db) = self.session_db().await else {
            return;
        };
        let mut keep_by_proj = HashMap::new();
        if let Ok(proj_ids) = db.list_project_config_proj_ids().await {
            for proj_id in proj_ids {
                if let Ok(Some(row)) = db.get_project_e2b_worker(proj_id).await {
                    keep_by_proj.insert(proj_id, row.sandbox_id);
                }
            }
        }
        match self.client.reap_cluster_warm_proj_orphans(&keep_by_proj).await {
            Ok(n) if n > 0 => info!(
                target: "claw_e2b_proj_worker",
                reaped = n,
                "reaped warm-proj orphan sandboxes after reconcile"
            ),
            Ok(_) => {}
            Err(e) => warn!(
                target: "claw_e2b_proj_worker",
                error = %e,
                "reap warm-proj orphans failed (best-effort)"
            ),
        }
    }

    /// Kill a rotated-out worker; log failures and reap same-proj orphans.
    async fn retire_worker_sandbox(&self, proj_id: i64, sandbox_id: &str) {
        if !self.client.sandbox_running(sandbox_id).await {
            return;
        }
        if let Err(e) = self.client.kill_sandbox(sandbox_id).await {
            warn!(
                target: "claw_e2b_proj_worker",
                proj_id,
                sandbox_id = %sandbox_id,
                error = %e,
                "kill rotated project worker failed — reaping warm-proj orphans"
            );
        }
        match self.client.reap_warm_proj_orphans(proj_id, "").await {
            Ok(n) if n > 0 => info!(
                target: "claw_e2b_proj_worker",
                proj_id,
                reaped = n,
                "reaped warm-proj orphans after retire"
            ),
            Ok(_) => {}
            Err(e) => warn!(
                target: "claw_e2b_proj_worker",
                proj_id,
                error = %e,
                "reap warm-proj orphans after retire failed"
            ),
        }
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

    /// Per-proj: skip if online + contract matches; rotate or create otherwise.
    pub async fn reconcile_proj(&self, proj_id: i64) -> Result<(), String> {
        let spec = self.desired_worker_spec(proj_id).await?;
        let db = self.session_db().await?;
        let desired_contract = desired_worker_contract(
            db.as_ref(),
            &spec.e2b_template_id,
            proj_id,
            &spec.profile_label,
        )
        .await?;
        let row = db
            .get_project_e2b_worker(proj_id)
            .await
            .map_err(|e| format!("get project_e2b_worker: {e}"))?;

        if let Some(ref existing) = row {
            let contract_ok = existing.template_id == desired_contract
                || !contract_requires_rotation(&existing.template_id, &desired_contract);
            if contract_ok && self.client.sandbox_running(&existing.sandbox_id).await {
                let handle = E2bSandboxClient::handle_from_json(&existing.handle_json)?;
                let handle = self.normalize_handle(handle, spec.include_ovs);
                let ovs_ok = !spec.include_ovs || self.relaxed_ovs_http_ok(&handle).await;
                if ovs_ok {
                    if existing.template_id != desired_contract {
                        let now_ms = chrono::Utc::now().timestamp_millis();
                        let mut updated = existing.clone();
                        updated.template_id = desired_contract.clone();
                        updated.updated_at_ms = now_ms;
                        db.upsert_project_e2b_worker(&updated)
                            .await
                            .map_err(|e| format!("upsert project_e2b_worker contract relabel: {e}"))?;
                        self.cache_worker(
                            proj_id,
                            handle,
                            updated.worker_id.clone(),
                            desired_contract.clone(),
                        )
                        .await;
                        info!(
                            target: "claw_e2b_proj_worker",
                            proj_id,
                            sandbox_id = %existing.sandbox_id,
                            old_contract = %existing.template_id,
                            new_contract = %desired_contract,
                            "proj worker contract relabeled (no sandbox rotation)"
                        );
                    } else {
                        self.cache_worker(
                            proj_id,
                            handle,
                            existing.worker_id.clone(),
                            existing.template_id.clone(),
                        )
                        .await;
                    }
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
                        profile = %spec.profile_label,
                        "proj worker online — skip create"
                    );
                    return Ok(());
                }
                warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    sandbox_id = %existing.sandbox_id,
                    "relaxed worker OVS unhealthy — rotate"
                );
            } else {
                info!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    old_sandbox = %existing.sandbox_id,
                    old_template = %existing.template_id,
                    new_template = %desired_contract,
                    "proj worker rotate (contract mismatch or offline)"
                );
            }
            self.retire_worker_sandbox(proj_id, &existing.sandbox_id).await;
            audit_rotation(
                db.as_ref(),
                WorkerRotationEvent {
                    proj_id,
                    event: "rotated_out".to_string(),
                    sandbox_id: Some(existing.sandbox_id.clone()),
                    worker_id: Some(existing.worker_id.clone()),
                    template_id: Some(existing.template_id.clone()),
                    reason: Some("contract_mismatch_or_offline".to_string()),
                    at_ms: chrono::Utc::now().timestamp_millis(),
                },
            )
            .await;
            db.delete_project_e2b_worker(proj_id)
                .await
                .map_err(|e| format!("delete project_e2b_worker: {e}"))?;
            self.workers.lock().await.remove(&proj_id);
        }

        self.create_and_persist(proj_id, &spec).await?;
        if let Ok(Some(row)) = db.get_project_e2b_worker(proj_id).await {
            match self
                .client
                .reap_warm_proj_orphans(proj_id, &row.sandbox_id)
                .await
            {
                Ok(n) if n > 0 => info!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    keep = %row.sandbox_id,
                    reaped = n,
                    "reaped warm-proj orphans after create"
                ),
                Ok(_) => {}
                Err(e) => warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    error = %e,
                    "reap warm-proj orphans after create failed"
                ),
            }
        }
        Ok(())
    }

    async fn create_and_persist(&self, proj_id: i64, spec: &WorkerSpec) -> Result<(), String> {
        let db = self.session_db().await?;
        let contract_key = desired_worker_contract(
            db.as_ref(),
            &spec.e2b_template_id,
            proj_id,
            &spec.profile_label,
        )
        .await?;
        let worker_id = allocate_worker_id();
        self.nas_layout
            .prepare_e2b_worker_bind_sources(db.as_ref(), proj_id, &worker_id)
            .await?;
        let handle = self
            .client
            .create_warm_proj_sandbox(
                &self.nas_layout.cluster_id()?,
                proj_id,
                &worker_id,
                &spec.e2b_template_id,
                spec.include_ovs,
            )
            .await?;
        if spec.include_ovs && !self.relaxed_ovs_http_ok(&handle).await {
            if let Err(kill_err) = self.client.kill_sandbox(&handle.sandbox_id).await {
                warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    sandbox_id = %handle.sandbox_id,
                    error = %kill_err,
                    "kill failed after OVS health check on new worker"
                );
            }
            return Err(format!(
                "relaxed worker sandbox {} created but built-in OVS :3000/ovs not reachable",
                handle.sandbox_id
            ));
        }
        if let Err(e) = self
            .client
            .renew_sandbox_ttl_secs(&handle.sandbox_id, self.worker_ttl_secs)
            .await
        {
            if let Err(kill_err) = self.client.kill_sandbox(&handle.sandbox_id).await {
                warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    sandbox_id = %handle.sandbox_id,
                    error = %kill_err,
                    "kill failed after TTL renew error on new worker"
                );
            }
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
            template_id = %spec.e2b_template_id,
            contract = %contract_key,
            profile = %spec.profile_label,
            builtin_ovs = spec.include_ovs,
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
        self.reconcile_proj(proj_id).await?;
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
                let proj_ids = match db.list_project_config_proj_ids().await {
                    Ok(ids) => ids,
                    Err(e) => {
                        warn!(target: "claw_e2b_proj_worker", error = %e, "renewal ticker: list proj_ids failed");
                        continue;
                    }
                };
                for proj_id in proj_ids {
                    if let Err(e) = self.reconcile_proj(proj_id).await {
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
        let spec = self.desired_worker_spec(proj_id).await?;
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
            self.retire_worker_sandbox(proj_id, &existing.sandbox_id).await;
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
            db.delete_project_e2b_worker(proj_id)
                .await
                .map_err(|e| format!("delete project_e2b_worker: {e}"))?;
            self.workers.lock().await.remove(&proj_id);
        }
        self.create_and_persist(proj_id, &spec).await?;
        if let Ok(Some(row)) = db.get_project_e2b_worker(proj_id).await {
            let _ = self
                .client
                .reap_warm_proj_orphans(proj_id, &row.sandbox_id)
                .await;
        }
        Ok(())
    }

    /// Gateway shutdown: workers survive (no kill).
    pub async fn shutdown_all(&self) {
        self.workers.lock().await.clear();
        self.leases.lock().await.clear();
        info!(target: "claw_e2b_proj_worker", "shutdown_all (workers left running on e2b)");
    }

    /// NAS `home/.vscode/settings.json` for OVS (`claw.projId`, `claw.clusterId`, …).
    pub async fn write_ovs_vscode_settings(
        &self,
        proj_id: i64,
        cluster_id: &str,
        worker_profile: &str,
    ) -> Result<(), String> {
        self.nas_layout
            .write_proj_claw_vscode_settings(cluster_id, proj_id, Some(worker_profile))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_contract_includes_project_home_rev_and_profile() {
        let key = worker_contract_key("claw-worker-relaxed", "2026-07-01_12-00-00", "relaxed");
        assert!(key.contains("nas-session-root-v3"));
        assert!(key.contains("#home=2026-07-01_12-00-00"));
        assert!(key.ends_with("#profile=relaxed"));
    }

    #[test]
    fn alias_vs_tpl_relaxed_relabels_without_rotation() {
        let alias = worker_contract_key(RELAXED_WORKER_ALIAS, "rev-1", "relaxed");
        let tpl = worker_contract_key("tpl_0153bc5c", "rev-1", "relaxed");
        assert!(!contract_requires_rotation(&alias, &tpl));
        assert!(!contract_requires_rotation(&tpl, &alias));
    }

    #[test]
    fn tpl_change_requires_rotation() {
        let a = worker_contract_key("tpl_aaaa", "rev-1", "strict");
        let b = worker_contract_key("tpl_bbbb", "rev-1", "strict");
        assert!(contract_requires_rotation(&a, &b));
    }

    #[test]
    fn home_rev_change_requires_rotation() {
        let a = worker_contract_key("tpl_aaaa", "rev-1", "strict");
        let b = worker_contract_key("tpl_aaaa", "rev-2", "strict");
        assert!(contract_requires_rotation(&a, &b));
    }
}
