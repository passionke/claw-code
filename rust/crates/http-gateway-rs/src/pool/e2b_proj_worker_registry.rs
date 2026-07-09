//! Per-project e2b worker registry — gateway-managed lifecycle (DB + e2b). Author: kejiqing
//!
//! Strict projects: N warm worker sandboxes per `proj_id` (PG `e2bWorker.poolSize`, default 4).
//! Relaxed: 1 worker with built-in OVS. Full-pool reconcile on startup / Admin poolSize change;
//! solve acquire picks one slot from memory and reconciles only on cache miss.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle, SANDBOX_LEASE_TICK_SECS};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::gateway_e2b_worker_settings::{
    e2b_project_worker_renew_interval_secs_from_env, e2b_project_worker_ttl_secs_from_env,
    load_e2b_strict_worker_pool_size, load_e2b_worker_relaxed_template_id,
    load_e2b_worker_template_id,
};
use crate::project_config_draft;
use crate::session_db::{
    e2b_worker_slot_i32, e2b_worker_slot_u32, GatewaySessionDb, ProjectFcWorkerRow,
    WorkerRotationEvent,
};

use super::config::relaxed_worker_allowed_from_env;
use super::e2b_nas_layout::allocate_worker_id;
use super::worker_profile::{
    default_worker_profile_json, effective_mode, profile_mode_label, WorkerProfileMode,
};
use super::NasLayoutBackend;

const PROJECT_WORKER_CONTRACT_VERSION: &str = "nas-session-root-v3";
/// e2b alias for relaxed worker; PG may store `tpl_*` for the same template.
const RELAXED_WORKER_ALIAS: &str = "claw-worker-relaxed";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct WorkerSlotKey {
    proj_id: i64,
    slot_index: u32,
}

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

fn template_rotation_needed(stored_tpl: &str, desired_tpl: &str) -> bool {
    if stored_tpl == desired_tpl {
        return false;
    }
    let stored_is_alias = stored_tpl == RELAXED_WORKER_ALIAS;
    let desired_is_alias = desired_tpl == RELAXED_WORKER_ALIAS;
    if stored_is_alias || desired_is_alias {
        return false;
    }
    if stored_tpl.starts_with("tpl_") && desired_tpl.starts_with("tpl_") {
        return stored_tpl != desired_tpl;
    }
    true
}

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

/// In-memory cache + per-slot lease ref-count.
pub struct E2bProjWorkerRegistry {
    client: Arc<E2bSandboxClient>,
    nas_layout: NasLayoutBackend,
    db: RwLock<Option<Arc<GatewaySessionDb>>>,
    workers: Mutex<HashMap<WorkerSlotKey, ProjWorkerRuntime>>,
    leases: Mutex<HashMap<WorkerSlotKey, u32>>,
    pending_retire: Mutex<HashSet<WorkerSlotKey>>,
    acquire_tie_break: AtomicUsize,
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
            pending_retire: Mutex::new(HashSet::new()),
            acquire_tie_break: AtomicUsize::new(0),
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

    async fn desired_pool_size(&self, proj_id: i64) -> Result<u32, String> {
        let db = self.session_db().await?;
        let json = db
            .get_worker_profile_json(proj_id)
            .await
            .unwrap_or_else(|_| default_worker_profile_json());
        let mode = effective_mode(relaxed_worker_allowed_from_env(), &json);
        match mode {
            WorkerProfileMode::Relaxed => Ok(1),
            WorkerProfileMode::Strict => load_e2b_strict_worker_pool_size(db.as_ref())
                .await
                .map_err(|e| format!("load e2bWorker poolSize: {e}")),
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

    fn normalize_handle(
        &self,
        mut handle: E2bSandboxHandle,
        include_ovs: bool,
    ) -> E2bSandboxHandle {
        if include_ovs {
            handle = E2bSandboxClient::handle_with_builtin_ovs(handle, &self.client);
        }
        handle
    }

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

    /// Best-effort reconcile every project (e.g. after Admin poolSize change).
    pub async fn reconcile_all_projects(&self) -> Result<(), String> {
        let db = self.session_db().await?;
        let proj_ids = db
            .list_project_config_proj_ids()
            .await
            .map_err(|e| format!("list project_config proj_ids: {e}"))?;
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
        Ok(())
    }

    async fn reap_cluster_warm_proj_orphans_best_effort(&self) {
        let Ok(db) = self.session_db().await else {
            return;
        };
        let mut keep_by_proj: HashMap<i64, Vec<String>> = HashMap::new();
        if let Ok(proj_ids) = db.list_project_config_proj_ids().await {
            for proj_id in proj_ids {
                if let Ok(rows) = db.list_project_e2b_workers(proj_id).await {
                    let ids: Vec<String> = rows.into_iter().map(|r| r.sandbox_id).collect();
                    if !ids.is_empty() {
                        keep_by_proj.insert(proj_id, ids);
                    }
                }
            }
        }
        match self
            .client
            .reap_cluster_warm_proj_orphans(&keep_by_proj)
            .await
        {
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
        let keep: Vec<String> = self
            .all_persisted_sandbox_ids()
            .await
            .into_iter()
            .filter(|id| id != sandbox_id)
            .collect();
        match self.client.reap_warm_proj_orphans(proj_id, &keep).await {
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

    pub async fn reconcile_proj(&self, proj_id: i64) -> Result<(), String> {
        let pool_size = self.desired_pool_size(proj_id).await?;
        for slot_index in 0..pool_size {
            self.reconcile_proj_slot(proj_id, slot_index).await?;
        }
        let db = self.session_db().await?;
        let existing = db
            .list_project_e2b_workers(proj_id)
            .await
            .map_err(|e| format!("list project_e2b_workers: {e}"))?;
        for row in existing {
            if e2b_worker_slot_u32(row.slot_index) >= pool_size {
                self.try_retire_slot(proj_id, e2b_worker_slot_u32(row.slot_index))
                    .await?;
            }
        }
        Ok(())
    }

    async fn try_retire_slot(&self, proj_id: i64, slot_index: u32) -> Result<(), String> {
        let key = WorkerSlotKey {
            proj_id,
            slot_index,
        };
        let active = self.active_leases(key).await;
        if active > 0 {
            self.pending_retire.lock().await.insert(key);
            return Ok(());
        }
        self.pending_retire.lock().await.remove(&key);
        let db = self.session_db().await?;
        let row = db
            .get_project_e2b_worker(proj_id, e2b_worker_slot_i32(slot_index))
            .await
            .map_err(|e| format!("get project_e2b_worker slot: {e}"))?;
        let Some(existing) = row else {
            self.workers.lock().await.remove(&key);
            return Ok(());
        };
        info!(
            target: "claw_e2b_proj_worker",
            proj_id,
            slot_index,
            sandbox_id = %existing.sandbox_id,
            "retire worker slot (pool shrink)"
        );
        self.retire_worker_sandbox(proj_id, &existing.sandbox_id)
            .await;
        db.delete_project_e2b_worker_slot(proj_id, e2b_worker_slot_i32(slot_index))
            .await
            .map_err(|e| format!("delete project_e2b_worker slot: {e}"))?;
        self.workers.lock().await.remove(&key);
        Ok(())
    }

    async fn reconcile_proj_slot(&self, proj_id: i64, slot_index: u32) -> Result<(), String> {
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
            .get_project_e2b_worker(proj_id, e2b_worker_slot_i32(slot_index))
            .await
            .map_err(|e| format!("get project_e2b_worker: {e}"))?;

        let key = WorkerSlotKey {
            proj_id,
            slot_index,
        };

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
                        db.upsert_project_e2b_worker(&updated).await.map_err(|e| {
                            format!("upsert project_e2b_worker contract relabel: {e}")
                        })?;
                        self.cache_worker(
                            key,
                            handle,
                            updated.worker_id.clone(),
                            desired_contract.clone(),
                        )
                        .await;
                    } else {
                        self.cache_worker(
                            key,
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
                    return Ok(());
                }
                warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    slot_index,
                    sandbox_id = %existing.sandbox_id,
                    "relaxed worker OVS unhealthy — rotate"
                );
            } else {
                info!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    slot_index,
                    old_sandbox = %existing.sandbox_id,
                    "proj worker rotate (contract mismatch or offline)"
                );
            }
            if self.active_leases(key).await > 0 {
                self.pending_retire.lock().await.insert(key);
                return Ok(());
            }
            self.retire_worker_sandbox(proj_id, &existing.sandbox_id)
                .await;
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
            db.delete_project_e2b_worker_slot(proj_id, e2b_worker_slot_i32(slot_index))
                .await
                .map_err(|e| format!("delete project_e2b_worker: {e}"))?;
            self.workers.lock().await.remove(&key);
        }

        self.create_and_persist_slot(proj_id, slot_index, &spec)
            .await?;
        if let Ok(Some(row)) = db
            .get_project_e2b_worker(proj_id, e2b_worker_slot_i32(slot_index))
            .await
        {
            let keep: Vec<String> = db
                .list_project_e2b_workers(proj_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|r| r.sandbox_id)
                .collect();
            let _ = self.client.reap_warm_proj_orphans(proj_id, &keep).await;
            let _ = row;
        }
        Ok(())
    }

    async fn create_and_persist_slot(
        &self,
        proj_id: i64,
        slot_index: u32,
        spec: &WorkerSpec,
    ) -> Result<(), String> {
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
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!(
                "relaxed worker sandbox {} created but built-in OVS :3000/ovs not reachable",
                handle.sandbox_id
            ));
        }
        self.client
            .renew_sandbox_ttl_secs(&handle.sandbox_id, self.worker_ttl_secs)
            .await
            .map_err(|e| format!("renew new project worker TTL: {e}"))?;

        let now_ms = chrono::Utc::now().timestamp_millis();
        let row = ProjectFcWorkerRow {
            proj_id,
            slot_index: e2b_worker_slot_i32(slot_index),
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

        let key = WorkerSlotKey {
            proj_id,
            slot_index,
        };
        self.cache_worker(key, handle.clone(), worker_id, contract_key.clone())
            .await;
        info!(
            target: "claw_e2b_proj_worker",
            proj_id,
            slot_index,
            sandbox_id = %handle.sandbox_id,
            contract = %contract_key,
            profile = %spec.profile_label,
            "proj worker slot created and persisted"
        );
        Ok(())
    }

    async fn cache_worker(
        &self,
        key: WorkerSlotKey,
        handle: E2bSandboxHandle,
        worker_id: String,
        template_id: String,
    ) {
        self.client.register_tracked_sandbox(&handle.sandbox_id);
        self.workers.lock().await.insert(
            key,
            ProjWorkerRuntime {
                handle,
                worker_id,
                template_id,
            },
        );
    }

    /// Ensure slot-0 worker (relaxed OVS / legacy callers).
    pub async fn ensure_worker(&self, proj_id: i64) -> Result<(E2bSandboxHandle, String), String> {
        self.reconcile_proj_slot(proj_id, 0).await?;
        let key = WorkerSlotKey {
            proj_id,
            slot_index: 0,
        };
        let guard = self.workers.lock().await;
        let rt = guard
            .get(&key)
            .ok_or_else(|| format!("proj worker missing after reconcile proj_{proj_id} slot 0"))?;
        Ok((rt.handle.clone(), rt.worker_id.clone()))
    }

    /// Strict solve: least-lease among pool slots. Relaxed: slot 0 only.
    ///
    /// Hot path: memory `pick_least_lease_slot` → `acquire_slot` for **one** slot only.
    /// Full-pool `reconcile_proj` runs on gateway startup / Admin poolSize change / background
    /// ticker — not on every solve acquire. Author: kejiqing
    pub async fn acquire_for_solve(
        &self,
        proj_id: i64,
        _session_id: &str,
    ) -> Result<(E2bSandboxHandle, String, u32), String> {
        let pool_size = self.desired_pool_size(proj_id).await?;
        if pool_size == 1 {
            let (handle, worker_id) = self.acquire_slot(proj_id, 0).await?;
            return Ok((handle, worker_id, 0));
        }
        let slot_index = self.pick_least_lease_slot(proj_id, pool_size).await?;
        let (handle, worker_id) = self.acquire_slot(proj_id, slot_index).await?;
        Ok((handle, worker_id, slot_index))
    }

    async fn pick_least_lease_slot(&self, proj_id: i64, pool_size: u32) -> Result<u32, String> {
        let workers = self.workers.lock().await;
        let leases = self.leases.lock().await;
        let present: Vec<u32> = workers
            .keys()
            .filter(|k| k.proj_id == proj_id)
            .map(|k| k.slot_index)
            .collect();
        let lease_by_slot: HashMap<u32, u32> = leases
            .iter()
            .filter(|(k, _)| k.proj_id == proj_id)
            .map(|(k, &n)| (k.slot_index, n))
            .collect();
        let tie = self.acquire_tie_break.fetch_add(1, Ordering::Relaxed);
        Ok(select_least_lease_slot(
            pool_size,
            &present,
            &lease_by_slot,
            tie as u32,
        ))
    }

    /// Relaxed interactive: slot 0 only.
    pub async fn acquire(&self, proj_id: i64) -> Result<(E2bSandboxHandle, String), String> {
        let (handle, worker_id, _) = self.acquire_for_solve(proj_id, "").await?;
        Ok((handle, worker_id))
    }

    async fn acquire_slot(
        &self,
        proj_id: i64,
        slot_index: u32,
    ) -> Result<(E2bSandboxHandle, String), String> {
        let key = WorkerSlotKey {
            proj_id,
            slot_index,
        };
        // Warm hit: in-memory handle + lease bump only (no e2b HTTP on acquire hot path).
        if let Some((handle, worker_id)) = {
            let guard = self.workers.lock().await;
            guard
                .get(&key)
                .map(|rt| (rt.handle.clone(), rt.worker_id.clone()))
        } {
            let mut leases = self.leases.lock().await;
            *leases.entry(key).or_insert(0) += 1;
            return Ok((handle, worker_id));
        }
        // Cache miss / missing slot: reconcile this slot only (create or PG→e2b probe).
        self.reconcile_proj_slot(proj_id, slot_index).await?;
        let guard = self.workers.lock().await;
        let rt = guard.get(&key).ok_or_else(|| {
            format!("proj worker missing after reconcile proj_{proj_id} slot {slot_index}")
        })?;
        let handle = rt.handle.clone();
        let worker_id = rt.worker_id.clone();
        drop(guard);
        let mut leases = self.leases.lock().await;
        *leases.entry(key).or_insert(0) += 1;
        Ok((handle, worker_id))
    }

    pub async fn release_slot(&self, proj_id: i64, slot_index: u32) {
        let key = WorkerSlotKey {
            proj_id,
            slot_index,
        };
        let mut leases = self.leases.lock().await;
        if let Some(n) = leases.get_mut(&key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                leases.remove(&key);
            }
        }
        drop(leases);
        if self.pending_retire.lock().await.contains(&key) && self.active_leases(key).await == 0 {
            let _ = self.try_retire_slot(proj_id, slot_index).await;
        }
    }

    /// Release slot 0 (relaxed interactive).
    pub async fn release(&self, proj_id: i64) {
        self.release_slot(proj_id, 0).await;
    }

    async fn active_leases(&self, key: WorkerSlotKey) -> u32 {
        self.leases.lock().await.get(&key).copied().unwrap_or(0)
    }

    #[must_use]
    pub async fn active_leases_for_slot(&self, proj_id: i64, slot_index: u32) -> u32 {
        self.active_leases(WorkerSlotKey {
            proj_id,
            slot_index,
        })
        .await
    }

    #[must_use]
    pub async fn leased_handle(&self, proj_id: i64) -> Option<E2bSandboxHandle> {
        self.workers
            .lock()
            .await
            .get(&WorkerSlotKey {
                proj_id,
                slot_index: 0,
            })
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

    /// Best-effort TTL touch for persisted workers (`spawn_lease_ticker` is primary at 60s).
    pub fn spawn_renewal_ticker(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(self.renew_interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let sandbox_ids = self.all_persisted_sandbox_ids().await;
                for sandbox_id in sandbox_ids {
                    if let Err(e) = self.client.touch_sandbox_lease(&sandbox_id).await {
                        warn!(
                            target: "claw_e2b_proj_worker",
                            sandbox_id = %sandbox_id,
                            error = %e,
                            "renewal ticker TTL touch failed"
                        );
                    }
                }
            }
        });
    }

    pub async fn force_rotate_proj(
        &self,
        proj_id: i64,
        slot_index: Option<u32>,
    ) -> Result<(), String> {
        let pool_size = self.desired_pool_size(proj_id).await?;
        let slots: Vec<u32> = match slot_index {
            Some(s) => vec![s],
            None => (0..pool_size).collect(),
        };
        for slot in slots {
            self.force_rotate_slot(proj_id, slot).await?;
        }
        Ok(())
    }

    async fn force_rotate_slot(&self, proj_id: i64, slot_index: u32) -> Result<(), String> {
        let db = self.session_db().await?;
        let spec = self.desired_worker_spec(proj_id).await?;
        let row = db
            .get_project_e2b_worker(proj_id, e2b_worker_slot_i32(slot_index))
            .await
            .map_err(|e| format!("get project_e2b_worker: {e}"))?;
        let key = WorkerSlotKey {
            proj_id,
            slot_index,
        };
        if let Some(ref existing) = row {
            if self.active_leases(key).await > 0 {
                return Err(format!(
                    "proj_{proj_id} slot {slot_index} has active leases; wait for turns to finish"
                ));
            }
            info!(
                target: "claw_e2b_proj_worker",
                proj_id,
                slot_index,
                sandbox_id = %existing.sandbox_id,
                "admin force rotate project worker slot"
            );
            self.retire_worker_sandbox(proj_id, &existing.sandbox_id)
                .await;
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
            db.delete_project_e2b_worker_slot(proj_id, e2b_worker_slot_i32(slot_index))
                .await
                .map_err(|e| format!("delete project_e2b_worker: {e}"))?;
            self.workers.lock().await.remove(&key);
        }
        self.create_and_persist_slot(proj_id, slot_index, &spec)
            .await?;
        Ok(())
    }

    pub async fn shutdown_all(&self) {
        self.workers.lock().await.clear();
        self.leases.lock().await.clear();
        self.pending_retire.lock().await.clear();
        info!(target: "claw_e2b_proj_worker", "shutdown_all (workers left running on e2b)");
    }

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

/// Pure least-lease slot picker (missing slot first, else min lease + tie-break).
fn select_least_lease_slot(
    pool_size: u32,
    present_slots: &[u32],
    lease_by_slot: &HashMap<u32, u32>,
    tie_break: u32,
) -> u32 {
    for slot_index in 0..pool_size {
        if !present_slots.contains(&slot_index) {
            return slot_index;
        }
    }
    let mut best_slot = 0u32;
    let mut best_count = u32::MAX;
    for slot_index in 0..pool_size {
        let count = *lease_by_slot.get(&slot_index).unwrap_or(&0);
        if count < best_count {
            best_count = count;
            best_slot = slot_index;
        }
    }
    (best_slot + (tie_break % pool_size)) % pool_size
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

    #[test]
    fn least_lease_prefers_missing_slot() {
        let present = vec![0, 1, 3];
        let leases = HashMap::from([(0, 1), (1, 0), (3, 0)]);
        assert_eq!(select_least_lease_slot(4, &present, &leases, 0), 2);
    }

    #[test]
    fn least_lease_picks_lowest_lease_count() {
        let present = vec![0, 1, 2, 3];
        let leases = HashMap::from([(0, 2), (1, 0), (2, 1), (3, 3)]);
        assert_eq!(select_least_lease_slot(4, &present, &leases, 0), 1);
    }

    #[test]
    fn least_lease_tie_break_is_deterministic() {
        let present = vec![0, 1];
        let leases = HashMap::from([(0, 0), (1, 0)]);
        assert_eq!(select_least_lease_slot(2, &present, &leases, 0), 0);
        assert_eq!(select_least_lease_slot(2, &present, &leases, 1), 1);
    }
}
