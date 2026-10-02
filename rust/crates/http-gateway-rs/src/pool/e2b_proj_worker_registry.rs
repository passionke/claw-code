//! Per-project e2b worker registry — gateway-managed lifecycle (DB + e2b). Author: kejiqing
//!
//! Strict projects: N warm worker sandboxes per `proj_id` (global `e2bWorker.poolSize` default 1,
//! optional per-project `worker_profile_json.poolSize`, capped by `CLAW_E2B_POOL_SIZE_CAP`).
//! Relaxed: 1 worker (permissions-only; home mount rw). Full-pool reconcile on startup / Admin
//! poolSize change; solve acquire picks one slot from memory and reconciles only on cache miss.
//!
//! Image / buildId: remote rebuild only updates PG. Healthy sandboxes are never killed because
//! buildId/templateId changed at runtime. New image is applied on gateway startup
//! (`image_refresh`), manual reset, or when the sandbox is dead/unhealthy.

use futures_util::StreamExt;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle, SANDBOX_LEASE_TICK_SECS};
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

use crate::gateway_e2b_lifecycle_decision::{
    decide_lifecycle_action, decide_scope_after_resume_failure, decide_scope_existing_worker,
    decide_scope_probe_only, lifecycle_probe_registry, scope_drop_detail,
    scope_invalidate_audit_reason, scope_sandbox_probe, worker_slot_probe_key, LifecycleAction,
    LifecycleDecisionInput, ProbeVerdict, ScopeWorkerAction, PROBE_MAX_ATTEMPTS,
};
use crate::gateway_e2b_worker_settings::{
    e2b_project_worker_renew_interval_secs_from_env, e2b_project_worker_ttl_secs_from_env,
    e2b_worker_relaxed_template_from_env, e2b_worker_template_from_env, load_e2b_worker_build_id,
    load_e2b_worker_relaxed_build_id, load_e2b_worker_relaxed_template_id,
    load_e2b_worker_template_id,
};
use crate::project_config_draft;
use crate::project_scope::{parse_scope_json, scope_worker_cap_from_env};
use crate::session_db::{
    e2b_worker_slot_i32, e2b_worker_slot_u32, GatewaySessionDb, ProjectFcWorkerRow,
    WorkerRotationEvent,
};
use serde_json::json;

use super::config::relaxed_worker_allowed_from_env;
use super::e2b_nas_layout::allocate_worker_id;
use super::worker_profile::{
    default_worker_profile_json, effective_mode, load_desired_worker_pool_size, profile_mode_label,
    WorkerProfileMode,
};
use super::NasLayoutBackend;

const PROJECT_WORKER_CONTRACT_VERSION: &str = "nas-session-root-v3";
/// Parallel sandbox switches during gateway startup. Author: kejiqing
const STARTUP_WORKER_SWITCH_CONCURRENCY: usize = 8;
/// Idle-pause ticker interval for scope workers. Author: kejiqing
const SCOPE_IDLE_PAUSE_TICK_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WorkerSlotKey {
    proj_id: i64,
    /// Empty = singleton project pool slot. Author: kejiqing
    scope_key: String,
    slot_index: u32,
}

fn singleton_slot_key(proj_id: i64, slot_index: u32) -> WorkerSlotKey {
    WorkerSlotKey {
        proj_id,
        scope_key: String::new(),
        slot_index,
    }
}

fn scope_slot_key(proj_id: i64, scope_key: &str, slot_index: u32) -> WorkerSlotKey {
    WorkerSlotKey {
        proj_id,
        scope_key: scope_key.to_string(),
        slot_index,
    }
}

/// Outcome of probing a scope worker before lease. Author: kejiqing
enum ScopeWorkerReady {
    Ready,
    /// PG row + cache cleared; caller must create.
    Dropped,
}

/// Split `{template}` or `{template}@{buildId}` head. Author: kejiqing
fn split_template_build(head: &str) -> (String, Option<String>) {
    match head.split_once('@') {
        Some((tpl, build)) if !build.trim().is_empty() => {
            (tpl.to_string(), Some(build.trim().to_string()))
        }
        _ => (head.to_string(), None),
    }
}

fn worker_contract_key(
    template_id: &str,
    build_id: Option<&str>,
    project_home_rev: &str,
    profile: &str,
) -> String {
    let head = match build_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(b) => format!("{template_id}@{b}"),
        None => template_id.to_string(),
    };
    format!("{head}#{PROJECT_WORKER_CONTRACT_VERSION}#home={project_home_rev}#profile={profile}")
}

async fn desired_worker_contract(
    db: &GatewaySessionDb,
    template_id: &str,
    build_id: Option<&str>,
    proj_id: i64,
    profile: &str,
) -> Result<String, String> {
    let home_rev = match project_config_draft::row_for_materialize(db, proj_id).await {
        Ok(Some(row)) => row.content_rev,
        Ok(None) => "none".to_string(),
        Err(e) => return Err(format!("load project home rev for worker contract: {e}")),
    };
    Ok(worker_contract_key(
        template_id,
        build_id,
        &home_rev,
        profile,
    ))
}

#[derive(Debug, PartialEq, Eq)]
struct WorkerContractParts {
    template: String,
    build_id: Option<String>,
    version: String,
    home_rev: String,
    profile: String,
}

fn parse_worker_contract(key: &str) -> Option<WorkerContractParts> {
    let mut parts = key.splitn(4, '#');
    let head = parts.next()?;
    let (template, build_id) = split_template_build(head);
    let version = parts.next()?.to_string();
    let home = parts.next()?.strip_prefix("home=")?.to_string();
    let profile = parts.next()?.strip_prefix("profile=")?.to_string();
    Some(WorkerContractParts {
        template,
        build_id,
        version,
        home_rev: home,
        profile,
    })
}

/// Home / profile / contract-version mismatch (not image). Template/build alone never rotate. Author: kejiqing
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
    stored_parts.version != desired_parts.version
        || stored_parts.home_rev != desired_parts.home_rev
        || stored_parts.profile != desired_parts.profile
}

/// Applied `@build` pin. Relabel may change the template name only when this matches. Author: kejiqing
fn applied_build_pin(key: &str) -> Option<String> {
    parse_worker_contract(key).and_then(|parts| {
        parts
            .build_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

/// True when the stored contract may be rewritten without recreating the sandbox.
/// A different build pin must stay recorded until startup actually switches. Author: kejiqing
fn same_applied_build(stored: &str, desired: &str) -> bool {
    let (Some(_stored_parts), Some(_desired_parts)) = (
        parse_worker_contract(stored),
        parse_worker_contract(desired),
    ) else {
        return false;
    };
    applied_build_pin(stored) == applied_build_pin(desired)
}

/// Startup / manual image window: desired build pin differs from applied. Author: kejiqing
fn image_build_refresh_needed(stored: &str, desired: &str) -> bool {
    let Some(desired_parts) = parse_worker_contract(desired) else {
        return true;
    };
    let Some(desired_build) = desired_parts
        .build_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        // No pin → never force image refresh for build alone.
        return false;
    };
    let Some(stored_parts) = parse_worker_contract(stored) else {
        return true;
    };
    match stored_parts
        .build_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        None => true, // legacy row without @build → align on startup
        Some(applied) => applied != desired_build,
    }
}

/// Unified recreate decision. `image_refresh` only on gateway startup (or callers that opt in).
/// Remote rebuild must not set this at runtime. Author: kejiqing
fn needs_recreate(stored: &str, desired: &str, image_refresh: bool, sandbox_alive: bool) -> bool {
    if !sandbox_alive {
        return true;
    }
    if contract_requires_rotation(stored, desired) {
        return true;
    }
    image_refresh && image_build_refresh_needed(stored, desired)
}

struct WorkerSpec {
    e2b_template_id: String,
    build_id: Option<String>,
    mode: WorkerProfileMode,
    profile_label: String,
}

impl WorkerSpec {
    /// Strict → home RO; Relaxed → home RW (permissions only). Author: kejiqing
    fn home_read_only(&self) -> bool {
        matches!(self.mode, WorkerProfileMode::Strict)
    }
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
                let build_id = load_e2b_worker_relaxed_build_id(db.as_ref())
                    .await
                    .map_err(|e| format!("load e2bWorkerRelaxed buildId: {e}"))?;
                Ok(WorkerSpec {
                    e2b_template_id,
                    build_id,
                    mode: WorkerProfileMode::Relaxed,
                    profile_label,
                })
            }
            WorkerProfileMode::Strict => {
                let e2b_template_id = load_e2b_worker_template_id(db.as_ref())
                    .await
                    .map_err(|e| format!("load e2bWorker template: {e}"))?;
                let build_id = load_e2b_worker_build_id(db.as_ref())
                    .await
                    .map_err(|e| format!("load e2bWorker buildId: {e}"))?;
                Ok(WorkerSpec {
                    e2b_template_id,
                    build_id,
                    mode: WorkerProfileMode::Strict,
                    profile_label,
                })
            }
        }
    }

    async fn desired_pool_size(&self, proj_id: i64) -> Result<u32, String> {
        let db = self.session_db().await?;
        load_desired_worker_pool_size(db.as_ref(), proj_id).await
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
            concurrency = STARTUP_WORKER_SWITCH_CONCURRENCY,
            "reconcile project e2b workers on startup"
        );
        let mut pools = Vec::with_capacity(proj_ids.len());
        let mut slots = Vec::new();
        for proj_id in proj_ids {
            let pool_size = self.desired_pool_size(proj_id).await?;
            pools.push((proj_id, pool_size));
            for slot_index in 0..pool_size {
                slots.push((proj_id, slot_index));
            }
        }
        let mut errors = Vec::new();
        let results: Vec<Result<(), String>> = futures_util::stream::iter(slots)
            .map(|(proj_id, slot_index)| self.reconcile_proj_slot(proj_id, slot_index, true))
            .buffer_unordered(STARTUP_WORKER_SWITCH_CONCURRENCY)
            .collect()
            .await;
        for result in results {
            if let Err(e) = result {
                errors.push(e);
            }
        }
        for (proj_id, pool_size) in pools {
            if let Err(e) = self.retire_overflow_slots(proj_id, pool_size).await {
                errors.push(format!("proj {proj_id}: {e}"));
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        self.reap_cluster_warm_proj_orphans_best_effort().await;
        self.seed_lease_tracking_from_db().await;
        Ok(())
    }

    /// Best-effort reconcile every project (e.g. after Admin poolSize change).
    /// Does **not** refresh image on buildId change (runtime path). Author: kejiqing
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
                if let Ok(rows) = db.list_project_e2b_singleton_workers(proj_id).await {
                    let ids: Vec<String> = rows.into_iter().map(|r| r.sandbox_id).collect();
                    if !ids.is_empty() {
                        keep_by_proj.insert(proj_id, ids);
                    }
                }
            }
        }
        match self
            .client
            .reap_cluster_warm_proj_orphans(
                &self.nas_layout.cluster_id().unwrap_or_default(),
                &keep_by_proj,
            )
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
        let cluster_id = self.nas_layout.cluster_id().unwrap_or_default();
        match self
            .client
            .reap_warm_proj_orphans(&cluster_id, proj_id, &keep)
            .await
        {
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
        self.reconcile_proj_with_image_refresh(proj_id, false).await
    }

    /// Gateway startup: allow buildId-driven image refresh. Author: kejiqing
    pub async fn reconcile_proj_image_refresh(&self, proj_id: i64) -> Result<(), String> {
        self.reconcile_proj_with_image_refresh(proj_id, true).await
    }

    async fn reconcile_proj_with_image_refresh(
        &self,
        proj_id: i64,
        image_refresh: bool,
    ) -> Result<(), String> {
        let pool_size = self.desired_pool_size(proj_id).await?;
        for slot_index in 0..pool_size {
            self.reconcile_proj_slot(proj_id, slot_index, image_refresh)
                .await?;
        }
        self.retire_overflow_slots(proj_id, pool_size).await
    }

    async fn retire_overflow_slots(&self, proj_id: i64, pool_size: u32) -> Result<(), String> {
        let db = self.session_db().await?;
        let existing = db
            .list_project_e2b_singleton_workers(proj_id)
            .await
            .map_err(|e| format!("list project_e2b_singleton_workers: {e}"))?;
        for row in existing {
            if e2b_worker_slot_u32(row.slot_index) >= pool_size {
                self.try_retire_slot(proj_id, e2b_worker_slot_u32(row.slot_index))
                    .await?;
            }
        }
        Ok(())
    }

    async fn try_retire_slot(&self, proj_id: i64, slot_index: u32) -> Result<(), String> {
        let db = self.session_db().await?;
        db.with_project_e2b_worker_slot_lock(proj_id, e2b_worker_slot_i32(slot_index), || async {
            self.try_retire_slot_locked(proj_id, slot_index).await
        })
        .await
    }

    async fn try_retire_slot_locked(&self, proj_id: i64, slot_index: u32) -> Result<(), String> {
        let key = singleton_slot_key(proj_id, slot_index);
        let active = self.active_leases(key.clone()).await;
        let db = self.session_db().await?;
        let pg_busy = db
            .project_e2b_worker_is_busy(proj_id, e2b_worker_slot_i32(slot_index))
            .await
            .map_err(|e| format!("project_e2b_worker_is_busy: {e}"))?;
        if active > 0 || pg_busy {
            self.pending_retire.lock().await.insert(key);
            return Ok(());
        }
        self.pending_retire.lock().await.remove(&key);
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

    async fn reconcile_proj_slot(
        &self,
        proj_id: i64,
        slot_index: u32,
        image_refresh: bool,
    ) -> Result<(), String> {
        let db = self.session_db().await?;
        db.with_project_e2b_worker_slot_lock(proj_id, e2b_worker_slot_i32(slot_index), || async {
            self.reconcile_proj_slot_locked(proj_id, slot_index, image_refresh)
                .await
        })
        .await
    }

    async fn reconcile_proj_slot_locked(
        &self,
        proj_id: i64,
        slot_index: u32,
        image_refresh: bool,
    ) -> Result<(), String> {
        let spec = self.desired_worker_spec(proj_id).await?;
        let db = self.session_db().await?;
        let desired_contract = desired_worker_contract(
            db.as_ref(),
            &spec.e2b_template_id,
            spec.build_id.as_deref(),
            proj_id,
            &spec.profile_label,
        )
        .await?;
        let row = db
            .get_project_e2b_worker(proj_id, e2b_worker_slot_i32(slot_index))
            .await
            .map_err(|e| format!("get project_e2b_worker: {e}"))?;

        let key = singleton_slot_key(proj_id, slot_index);

        if let Some(ref existing) = row {
            let sandbox_alive = self.client.sandbox_running(&existing.sandbox_id).await
                || self.client.sandbox_paused(&existing.sandbox_id).await;
            let must_recreate = needs_recreate(
                &existing.template_id,
                &desired_contract,
                image_refresh,
                sandbox_alive,
            );
            if !must_recreate {
                if existing.lifecycle_state == "sleeping"
                    || self.client.sandbox_paused(&existing.sandbox_id).await
                {
                    let handle = self
                        .client
                        .resume_sandbox(&existing.sandbox_id, self.worker_ttl_secs)
                        .await
                        .map_err(|e| format!("resume paused project worker: {e}"))?;
                    let handle_json = E2bSandboxClient::handle_to_json(&handle);
                    db.update_project_e2b_worker_lifecycle(
                        proj_id,
                        "",
                        e2b_worker_slot_i32(slot_index),
                        "running",
                        Some(&handle_json),
                    )
                    .await
                    .map_err(|e| format!("update lifecycle after resume: {e}"))?;
                    self.cache_worker(
                        key,
                        handle,
                        existing.worker_id.clone(),
                        existing.template_id.clone(),
                    )
                    .await;
                    return Ok(());
                }
                let handle = E2bSandboxClient::handle_from_json(&existing.handle_json)?;
                if existing.template_id != desired_contract
                    && same_applied_build(&existing.template_id, &desired_contract)
                {
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    let mut updated = existing.clone();
                    updated.template_id = desired_contract.clone();
                    updated.updated_at_ms = now_ms;
                    db.upsert_project_e2b_worker(&updated)
                        .await
                        .map_err(|e| format!("upsert project_e2b_worker contract relabel: {e}"))?;
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
            info!(
                target: "claw_e2b_proj_worker",
                proj_id,
                slot_index,
                old_sandbox = %existing.sandbox_id,
                image_refresh,
                "proj worker rotate (contract / image_refresh / offline)"
            );
            let pg_busy = db
                .project_e2b_worker_is_busy(proj_id, e2b_worker_slot_i32(slot_index))
                .await
                .map_err(|e| format!("project_e2b_worker_is_busy: {e}"))?;
            if self.active_leases(key.clone()).await > 0 || pg_busy {
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

        self.create_and_persist_slot(proj_id, "", slot_index, &spec)
            .await?;
        if let Ok(Some(row)) = db
            .get_project_e2b_worker(proj_id, e2b_worker_slot_i32(slot_index))
            .await
        {
            let keep: Vec<String> = db
                .list_project_e2b_singleton_workers(proj_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|r| r.sandbox_id)
                .collect();
            let cluster_id = self.nas_layout.cluster_id().unwrap_or_default();
            let _ = self
                .client
                .reap_warm_proj_orphans(&cluster_id, proj_id, &keep)
                .await;
            let _ = row;
        }
        Ok(())
    }

    async fn create_and_persist_slot(
        &self,
        proj_id: i64,
        scope_key: &str,
        slot_index: u32,
        spec: &WorkerSpec,
    ) -> Result<(), String> {
        let db = self.session_db().await?;
        let contract_key = desired_worker_contract(
            db.as_ref(),
            &spec.e2b_template_id,
            spec.build_id.as_deref(),
            proj_id,
            &spec.profile_label,
        )
        .await?;
        let worker_id = allocate_worker_id();
        self.nas_layout
            .prepare_e2b_worker_bind_sources(db.as_ref(), proj_id, &worker_id)
            .await?;
        let worker_env_json = db
            .get_worker_env_json(proj_id)
            .await
            .map_err(|e| format!("load worker_env_json for proj {proj_id}: {e}"))?;
        let env_vars = crate::pool::parse_worker_env_map(&worker_env_json)
            .map_err(|e| format!("invalid worker_env_json for proj {proj_id}: {e}"))?;
        // Prefer alias for create target; pin buildId only when PG has one (after publish on
        // *this* e2b). Endpoint change clears pins so stale UUIDs cannot 503. Author: kejiqing
        let create_alias = match spec.mode {
            WorkerProfileMode::Relaxed => e2b_worker_relaxed_template_from_env(),
            WorkerProfileMode::Strict => e2b_worker_template_from_env(),
        };
        let template_ref = claw_e2b_sandbox_client::e2b_sandbox_template_ref(
            &create_alias,
            spec.build_id.as_deref(),
        );
        let handle = self
            .client
            .create_warm_proj_sandbox(
                &self.nas_layout.cluster_id()?,
                proj_id,
                &worker_id,
                &template_ref,
                spec.home_read_only(),
                env_vars,
            )
            .await?;
        self.client
            .renew_sandbox_ttl_secs(&handle.sandbox_id, self.worker_ttl_secs)
            .await
            .map_err(|e| format!("renew new project worker TTL: {e}"))?;

        let now_ms = chrono::Utc::now().timestamp_millis();
        let row = ProjectFcWorkerRow {
            proj_id,
            scope_key: scope_key.to_string(),
            slot_index: e2b_worker_slot_i32(slot_index),
            sandbox_id: handle.sandbox_id.clone(),
            worker_id: worker_id.clone(),
            template_id: contract_key.clone(),
            handle_json: E2bSandboxClient::handle_to_json(&handle),
            updated_at_ms: now_ms,
            in_use_count: 0,
            in_use_until_ms: 0,
            lifecycle_state: "running".to_string(),
            last_idle_at_ms: 0,
            mcp_bind_json: json!({}),
            invalid_reason: String::new(),
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
                reason: if scope_key.is_empty() {
                    None
                } else {
                    Some(format!("scope_key={scope_key}"))
                },
                at_ms: now_ms,
            },
        )
        .await;

        let key = scope_slot_key(proj_id, scope_key, slot_index);
        self.cache_worker(key, handle.clone(), worker_id, contract_key.clone())
            .await;
        info!(
            target: "claw_e2b_proj_worker",
            proj_id,
            scope_key = %scope_key,
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

    /// Ensure slot-0 worker (relaxed / legacy callers).
    pub async fn ensure_worker(&self, proj_id: i64) -> Result<(E2bSandboxHandle, String), String> {
        self.reconcile_proj_slot(proj_id, 0, false).await?;
        let key = singleton_slot_key(proj_id, 0);
        let guard = self.workers.lock().await;
        let rt = guard
            .get(&key)
            .ok_or_else(|| format!("proj worker missing after reconcile proj_{proj_id} slot 0"))?;
        Ok((rt.handle.clone(), rt.worker_id.clone()))
    }

    /// Strict solve: least-lease among pool slots. Relaxed: slot 0 only.
    ///
    /// Hot path: memory `pick_least_lease_slot` → `acquire_slot` for **one** slot only.
    /// Full-pool `reconcile_proj` runs on gateway startup / Admin poolSize change — not on
    /// background TTL ticker (that only touches leases). Author: kejiqing
    pub async fn acquire_for_solve(
        &self,
        proj_id: i64,
        _session_id: &str,
    ) -> Result<(E2bSandboxHandle, String, u32), String> {
        let db = self.session_db().await?;
        let role = db
            .get_project_role(proj_id)
            .await
            .map_err(|e| format!("get_project_role: {e}"))?;
        if role == crate::master_observer::PROJECT_ROLE_SCOPE {
            return Err(
                "proj role=scope must use acquire_for_scope_solve (not singleton pool acquire)"
                    .into(),
            );
        }
        let pool_size = self.desired_pool_size(proj_id).await?;
        // poolSize=0: on-demand create slot 0 (no warm pool). Author: kejiqing
        if pool_size == 0 {
            let (handle, worker_id) = self.acquire_slot(proj_id, 0).await?;
            return Ok((handle, worker_id, 0));
        }
        if pool_size == 1 {
            let (handle, worker_id) = self.acquire_slot(proj_id, 0).await?;
            return Ok((handle, worker_id, 0));
        }
        let slot_index = self.pick_least_lease_slot(proj_id, pool_size).await?;
        let (handle, worker_id) = self.acquire_slot(proj_id, slot_index).await?;
        Ok((handle, worker_id, slot_index))
    }

    /// Scope-role solve: one worker per `(proj_id, scope_key)` at slot 0. Author: kejiqing
    ///
    /// Dead / unresumable sandboxes are treated as missing: drop PG row + cache, then create
    /// (same end state as admin retire → next solve creates). Does not copy warm reconcile.
    pub async fn acquire_for_scope_solve(
        &self,
        proj_id: i64,
        scope_key: &str,
    ) -> Result<(E2bSandboxHandle, String, u32), String> {
        if scope_key.trim().is_empty() {
            return Err("scope_key must be non-empty for scope solve".into());
        }
        let db = self.session_db().await?;
        let key = scope_slot_key(proj_id, scope_key, 0);

        // Cache hit: verify running / resume if paused; dead → drop and fall through to create.
        {
            let guard = self.workers.lock().await;
            if let Some(rt) = guard.get(&key) {
                let sandbox_id = rt.handle.sandbox_id.clone();
                drop(guard);
                match self
                    .ensure_scope_worker_ready(proj_id, scope_key, 0, &sandbox_id)
                    .await?
                {
                    ScopeWorkerReady::Ready => {
                        let guard = self.workers.lock().await;
                        let rt = guard.get(&key).ok_or_else(|| {
                            format!(
                                "scope worker missing after warm verify proj_{proj_id} scope={scope_key}"
                            )
                        })?;
                        let handle = rt.handle.clone();
                        let worker_id = rt.worker_id.clone();
                        drop(guard);
                        self.bump_scope_lease(proj_id, scope_key, 0).await;
                        return Ok((handle, worker_id, 0));
                    }
                    ScopeWorkerReady::Dropped => {}
                }
            }
        }

        let row = db
            .get_project_e2b_worker_scoped(proj_id, scope_key, 0)
            .await
            .map_err(|e| format!("get project_e2b_worker scoped: {e}"))?;

        if let Some(existing) = row {
            let paused = self.client.sandbox_paused(&existing.sandbox_id).await;
            let running = if paused {
                false
            } else {
                self.client.sandbox_running(&existing.sandbox_id).await
            };
            let probe = scope_sandbox_probe(paused, running);
            match decide_scope_existing_worker(&existing.lifecycle_state, probe) {
                ScopeWorkerAction::Reuse => {
                    let handle = E2bSandboxClient::handle_from_json(&existing.handle_json)?;
                    self.cache_worker(
                        key.clone(),
                        handle.clone(),
                        existing.worker_id.clone(),
                        existing.template_id.clone(),
                    )
                    .await;
                    self.bump_scope_lease(proj_id, scope_key, 0).await;
                    return Ok((handle, existing.worker_id, 0));
                }
                ScopeWorkerAction::Resume => {
                    match self
                        .client
                        .resume_sandbox(&existing.sandbox_id, self.worker_ttl_secs)
                        .await
                    {
                        Ok(handle) => {
                            let handle_json = E2bSandboxClient::handle_to_json(&handle);
                            db.update_project_e2b_worker_lifecycle(
                                proj_id,
                                scope_key,
                                0,
                                "running",
                                Some(&handle_json),
                            )
                            .await
                            .map_err(|e| format!("update scope lifecycle after resume: {e}"))?;
                            self.cache_worker(
                                key.clone(),
                                handle.clone(),
                                existing.worker_id.clone(),
                                existing.template_id.clone(),
                            )
                            .await;
                            self.bump_scope_lease(proj_id, scope_key, 0).await;
                            return Ok((handle, existing.worker_id, 0));
                        }
                        Err(e) => {
                            debug_assert_eq!(
                                decide_scope_after_resume_failure(),
                                ScopeWorkerAction::Invalidate
                            );
                            self.invalidate_dead_scope_worker(
                                proj_id,
                                scope_key,
                                0,
                                &existing.sandbox_id,
                                &existing.worker_id,
                                &existing.template_id,
                                &format!("resume_failed:{e}"),
                            )
                            .await?;
                        }
                    }
                }
                ScopeWorkerAction::Invalidate => {
                    let detail = scope_drop_detail(&existing.lifecycle_state, probe);
                    self.invalidate_dead_scope_worker(
                        proj_id,
                        scope_key,
                        0,
                        &existing.sandbox_id,
                        &existing.worker_id,
                        &existing.template_id,
                        detail,
                    )
                    .await?;
                }
            }
        }

        // Missing (or just soft-invalidated): enforce cap then create (upsert clears invalid).
        let count = db
            .count_project_e2b_scope_workers(proj_id)
            .await
            .map_err(|e| format!("count scope workers: {e}"))?;
        let cap = i64::from(scope_worker_cap_from_env());
        if count >= cap {
            return Err(format!(
                "CLAW_E2B_SCOPE_WORKER_CAP={cap} reached for proj_{proj_id} (have {count})"
            ));
        }
        let spec = self.desired_worker_spec(proj_id).await?;
        self.create_and_persist_slot(proj_id, scope_key, 0, &spec)
            .await?;
        let guard = self.workers.lock().await;
        let rt = guard.get(&key).ok_or_else(|| {
            format!("scope worker missing after create proj_{proj_id} scope={scope_key}")
        })?;
        let handle = rt.handle.clone();
        let worker_id = rt.worker_id.clone();
        drop(guard);
        self.bump_scope_lease(proj_id, scope_key, 0).await;
        Ok((handle, worker_id, 0))
    }

    async fn bump_scope_lease(&self, proj_id: i64, scope_key: &str, slot_index: u32) {
        let key = scope_slot_key(proj_id, scope_key, slot_index);
        let mut leases = self.leases.lock().await;
        *leases.entry(key).or_insert(0) += 1;
        drop(leases);
        if let Ok(db) = self.session_db().await {
            let _ = db
                .bump_project_e2b_worker_in_use_scoped(
                    proj_id,
                    scope_key,
                    e2b_worker_slot_i32(slot_index),
                    30 * 60 * 1000,
                )
                .await;
        }
    }

    async fn pick_least_lease_slot(&self, proj_id: i64, pool_size: u32) -> Result<u32, String> {
        let workers = self.workers.lock().await;
        let leases = self.leases.lock().await;
        let present: Vec<u32> = workers
            .keys()
            .filter(|k| k.proj_id == proj_id && k.scope_key.is_empty())
            .map(|k| k.slot_index)
            .collect();
        let lease_by_slot: HashMap<u32, u32> = leases
            .iter()
            .filter(|(k, _)| k.proj_id == proj_id && k.scope_key.is_empty())
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

    async fn ensure_warm_worker_running(
        &self,
        proj_id: i64,
        slot_index: u32,
        sandbox_id: &str,
    ) -> Result<(), String> {
        if self.client.sandbox_paused(sandbox_id).await {
            let handle = self
                .client
                .resume_sandbox(sandbox_id, self.worker_ttl_secs)
                .await
                .map_err(|e| format!("resume paused warm worker: {e}"))?;
            if let Ok(db) = self.session_db().await {
                let handle_json = E2bSandboxClient::handle_to_json(&handle);
                let _ = db
                    .update_project_e2b_worker_lifecycle(
                        proj_id,
                        "",
                        e2b_worker_slot_i32(slot_index),
                        "running",
                        Some(&handle_json),
                    )
                    .await;
            }
            let key = singleton_slot_key(proj_id, slot_index);
            if let Some(rt) = self.workers.lock().await.get_mut(&key) {
                rt.handle = handle;
            }
            return Ok(());
        }
        let probe_key = worker_slot_probe_key(proj_id, slot_index);
        let registry = lifecycle_probe_registry();
        let now = chrono::Utc::now().timestamp_millis();
        let (last_ok, consecutive) = registry.snapshot(&probe_key);
        let (cache_action, _) = decide_lifecycle_action(&LifecycleDecisionInput {
            now_ms: now,
            last_ok_ms: last_ok,
            consecutive_failures: consecutive,
            probe_verdict: ProbeVerdict::NotRunning,
            force_recreate: false,
            busy: false,
        });
        if cache_action == LifecycleAction::ReuseSkipProbe {
            return Ok(());
        }
        let max = PROBE_MAX_ATTEMPTS.max(1);
        let mut running = false;
        for attempt in 1..=max {
            if self.client.sandbox_running(sandbox_id).await {
                running = true;
                break;
            }
            if attempt < max {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        if running {
            if self.client.envd_echo_reachable(sandbox_id).await {
                registry.record_success(&probe_key, now);
                return Ok(());
            }
            warn!(
                target: "claw_e2b_proj_worker",
                proj_id,
                slot_index,
                sandbox_id,
                "e2b API reports running but envd is unreachable; recreating worker"
            );
        }
        self.reconcile_proj_slot(proj_id, slot_index, false).await
    }

    /// Verify scope worker is usable; on dead/unresumable drop PG+cache as missing. Author: kejiqing
    async fn ensure_scope_worker_ready(
        &self,
        proj_id: i64,
        scope_key: &str,
        slot_index: u32,
        sandbox_id: &str,
    ) -> Result<ScopeWorkerReady, String> {
        let paused = self.client.sandbox_paused(sandbox_id).await;
        let running = if paused {
            false
        } else {
            self.client.sandbox_running(sandbox_id).await
        };
        let probe = scope_sandbox_probe(paused, running);
        match decide_scope_probe_only(probe) {
            ScopeWorkerAction::Reuse => Ok(ScopeWorkerReady::Ready),
            ScopeWorkerAction::Resume => {
                match self
                    .client
                    .resume_sandbox(sandbox_id, self.worker_ttl_secs)
                    .await
                {
                    Ok(handle) => {
                        if let Ok(db) = self.session_db().await {
                            let handle_json = E2bSandboxClient::handle_to_json(&handle);
                            let _ = db
                                .update_project_e2b_worker_lifecycle(
                                    proj_id,
                                    scope_key,
                                    e2b_worker_slot_i32(slot_index),
                                    "running",
                                    Some(&handle_json),
                                )
                                .await;
                        }
                        let key = scope_slot_key(proj_id, scope_key, slot_index);
                        if let Some(rt) = self.workers.lock().await.get_mut(&key) {
                            rt.handle = handle;
                        }
                        Ok(ScopeWorkerReady::Ready)
                    }
                    Err(e) => {
                        debug_assert_eq!(
                            decide_scope_after_resume_failure(),
                            ScopeWorkerAction::Invalidate
                        );
                        let (worker_id, template_id) = self
                            .scope_worker_ids_from_cache_or_db(proj_id, scope_key, slot_index)
                            .await;
                        self.invalidate_dead_scope_worker(
                            proj_id,
                            scope_key,
                            slot_index,
                            sandbox_id,
                            &worker_id,
                            &template_id,
                            &format!("resume_paused_failed:{e}"),
                        )
                        .await?;
                        Ok(ScopeWorkerReady::Dropped)
                    }
                }
            }
            ScopeWorkerAction::Invalidate => {
                let (worker_id, template_id) = self
                    .scope_worker_ids_from_cache_or_db(proj_id, scope_key, slot_index)
                    .await;
                self.invalidate_dead_scope_worker(
                    proj_id,
                    scope_key,
                    slot_index,
                    sandbox_id,
                    &worker_id,
                    &template_id,
                    scope_drop_detail("running", probe),
                )
                .await?;
                Ok(ScopeWorkerReady::Dropped)
            }
        }
    }

    async fn scope_worker_ids_from_cache_or_db(
        &self,
        proj_id: i64,
        scope_key: &str,
        slot_index: u32,
    ) -> (String, String) {
        let key = scope_slot_key(proj_id, scope_key, slot_index);
        if let Some(rt) = self.workers.lock().await.get(&key) {
            return (rt.worker_id.clone(), rt.template_id.clone());
        }
        if let Ok(db) = self.session_db().await {
            if let Ok(Some(row)) = db
                .get_project_e2b_worker_scoped(proj_id, scope_key, e2b_worker_slot_i32(slot_index))
                .await
            {
                return (row.worker_id, row.template_id);
            }
        }
        (String::new(), String::new())
    }

    /// Soft-invalidate dead scope worker so acquire can create (retain row for RCA). Author: kejiqing
    async fn invalidate_dead_scope_worker(
        &self,
        proj_id: i64,
        scope_key: &str,
        slot_index: u32,
        sandbox_id: &str,
        worker_id: &str,
        template_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        warn!(
            target: "claw_e2b_proj_worker",
            proj_id,
            scope_key = %scope_key,
            sandbox_id = %sandbox_id,
            reason = %reason,
            "scope worker dead/unresumable; invalidating PG row for recreate"
        );
        self.retire_worker_sandbox(proj_id, sandbox_id).await;
        let db = self.session_db().await?;
        let audit = scope_invalidate_audit_reason(reason, scope_key);
        audit_rotation(
            db.as_ref(),
            WorkerRotationEvent {
                proj_id,
                event: "invalidated".to_string(),
                sandbox_id: Some(sandbox_id.to_string()),
                worker_id: if worker_id.is_empty() {
                    None
                } else {
                    Some(worker_id.to_string())
                },
                template_id: if template_id.is_empty() {
                    None
                } else {
                    Some(template_id.to_string())
                },
                reason: Some(audit.clone()),
                at_ms: chrono::Utc::now().timestamp_millis(),
            },
        )
        .await;
        db.invalidate_project_e2b_worker_slot_scoped(
            proj_id,
            scope_key,
            e2b_worker_slot_i32(slot_index),
            &audit,
        )
        .await
        .map_err(|e| format!("invalidate dead scope worker: {e}"))?;
        let key = scope_slot_key(proj_id, scope_key, slot_index);
        self.workers.lock().await.remove(&key);
        self.leases.lock().await.remove(&key);
        Ok(())
    }

    async fn acquire_slot(
        &self,
        proj_id: i64,
        slot_index: u32,
    ) -> Result<(E2bSandboxHandle, String), String> {
        let key = singleton_slot_key(proj_id, slot_index);
        let warm_hit = {
            let guard = self.workers.lock().await;
            if let Some(rt) = guard.get(&key) {
                let sandbox_id = rt.handle.sandbox_id.clone();
                drop(guard);
                self.ensure_warm_worker_running(proj_id, slot_index, &sandbox_id)
                    .await?;
                true
            } else {
                false
            }
        };
        if warm_hit {
            let guard = self.workers.lock().await;
            let rt = guard.get(&key).ok_or_else(|| {
                format!("proj worker missing after warm verify proj_{proj_id} slot {slot_index}")
            })?;
            let handle = rt.handle.clone();
            let worker_id = rt.worker_id.clone();
            drop(guard);
            let mut leases = self.leases.lock().await;
            *leases.entry(key).or_insert(0) += 1;
            if let Ok(db) = self.session_db().await {
                let _ = db
                    .bump_project_e2b_worker_in_use(
                        proj_id,
                        e2b_worker_slot_i32(slot_index),
                        30 * 60 * 1000,
                    )
                    .await;
            }
            return Ok((handle, worker_id));
        }
        // Cache miss / missing slot: reconcile this slot only (create or PG→e2b probe).
        // Runtime path: no image_refresh (remote rebuild must not rotate). Author: kejiqing
        self.reconcile_proj_slot(proj_id, slot_index, false).await?;
        let guard = self.workers.lock().await;
        let rt = guard.get(&key).ok_or_else(|| {
            format!("proj worker missing after reconcile proj_{proj_id} slot {slot_index}")
        })?;
        let handle = rt.handle.clone();
        let worker_id = rt.worker_id.clone();
        drop(guard);
        let mut leases = self.leases.lock().await;
        *leases.entry(key).or_insert(0) += 1;
        if let Ok(db) = self.session_db().await {
            let _ = db
                .bump_project_e2b_worker_in_use(
                    proj_id,
                    e2b_worker_slot_i32(slot_index),
                    30 * 60 * 1000,
                )
                .await;
        }
        Ok((handle, worker_id))
    }

    pub async fn release_slot(&self, proj_id: i64, slot_index: u32, scope_key: &str) {
        let key = scope_slot_key(proj_id, scope_key, slot_index);
        let mut leases = self.leases.lock().await;
        if let Some(n) = leases.get_mut(&key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                leases.remove(&key);
            }
        }
        drop(leases);
        if let Ok(db) = self.session_db().await {
            let _ = db
                .release_project_e2b_worker_in_use_scoped(
                    proj_id,
                    scope_key,
                    e2b_worker_slot_i32(slot_index),
                )
                .await;
        }
        let leases_left = self.active_leases(key.clone()).await;
        if leases_left == 0 && scope_key.is_empty() {
            let pending = self.pending_retire.lock().await.contains(&key);
            let desired_zero = self
                .desired_pool_size(proj_id)
                .await
                .map(|n| n == 0)
                .unwrap_or(false);
            if pending || desired_zero {
                let _ = self.try_retire_slot(proj_id, slot_index).await;
            }
        }
    }

    /// Release slot 0 (relaxed interactive).
    pub async fn release(&self, proj_id: i64) {
        self.release_slot(proj_id, 0, "").await;
    }

    async fn active_leases(&self, key: WorkerSlotKey) -> u32 {
        self.leases.lock().await.get(&key).copied().unwrap_or(0)
    }

    #[must_use]
    pub async fn active_leases_for_slot(&self, proj_id: i64, slot_index: u32) -> u32 {
        self.active_leases(singleton_slot_key(proj_id, slot_index))
            .await
    }

    #[must_use]
    pub async fn leased_handle(&self, proj_id: i64) -> Option<E2bSandboxHandle> {
        self.workers
            .lock()
            .await
            .get(&singleton_slot_key(proj_id, 0))
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

    async fn singleton_persisted_sandbox_ids(&self) -> Vec<String> {
        if let Some(db) = self.db.read().await.clone() {
            if let Ok(ids) = db.list_project_e2b_singleton_sandbox_ids().await {
                return ids;
            }
        }
        self.workers
            .lock()
            .await
            .iter()
            .filter(|(k, _)| k.scope_key.is_empty())
            .map(|(_, rt)| rt.handle.sandbox_id.clone())
            .collect()
    }

    /// Best-effort TTL touch for singleton workers (`spawn_lease_ticker` is primary at 60s).
    /// Does not run full `reconcile_proj`. Author: kejiqing
    pub fn spawn_renewal_ticker(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(self.renew_interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let sandbox_ids = self.singleton_persisted_sandbox_ids().await;
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

    /// Pause idle scope workers when past per-project `idleSleepSecs`. Author: kejiqing
    pub fn spawn_scope_idle_pause_ticker(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(SCOPE_IDLE_PAUSE_TICK_SECS));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = self.pause_idle_scope_workers_once().await {
                    warn!(
                        target: "claw_e2b_proj_worker",
                        error = %e,
                        "scope idle pause ticker failed"
                    );
                }
            }
        });
    }

    async fn pause_idle_scope_workers_once(&self) -> Result<(), String> {
        let db = self.session_db().await?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        // Pass now so SQL returns candidates with last_idle_at_ms > 0; threshold checked below.
        let candidates = db
            .list_idle_scope_workers_for_pause(now_ms)
            .await
            .map_err(|e| format!("list_idle_scope_workers_for_pause: {e}"))?;
        for row in candidates {
            if row.in_use_count > 0 {
                continue;
            }
            let key = scope_slot_key(
                row.proj_id,
                &row.scope_key,
                e2b_worker_slot_u32(row.slot_index),
            );
            if self.active_leases(key.clone()).await > 0 {
                continue;
            }
            let scope_json = db
                .get_scope_json(row.proj_id)
                .await
                .map_err(|e| format!("get_scope_json: {e}"))?;
            let Ok(cfg) = parse_scope_json(&scope_json) else {
                continue;
            };
            if row.last_idle_at_ms <= 0 {
                continue;
            }
            let idle_ms = now_ms.saturating_sub(row.last_idle_at_ms);
            if idle_ms < cfg.idle_sleep_ms() {
                continue;
            }
            if let Err(e) = self.client.pause_sandbox(&row.sandbox_id).await {
                warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id = row.proj_id,
                    scope_key = %row.scope_key,
                    sandbox_id = %row.sandbox_id,
                    error = %e,
                    "pause idle scope worker failed"
                );
                continue;
            }
            if let Err(e) = db
                .update_project_e2b_worker_lifecycle(
                    row.proj_id,
                    &row.scope_key,
                    row.slot_index,
                    "sleeping",
                    None,
                )
                .await
            {
                warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id = row.proj_id,
                    scope_key = %row.scope_key,
                    error = %e,
                    "set lifecycle sleeping after pause failed"
                );
            }
            self.workers.lock().await.remove(&key);
            info!(
                target: "claw_e2b_proj_worker",
                proj_id = row.proj_id,
                scope_key = %row.scope_key,
                sandbox_id = %row.sandbox_id,
                idle_ms,
                "paused idle scope worker"
            );
        }
        Ok(())
    }

    /// Admin force reset: always kill + create (manual recreate window). Author: kejiqing
    ///
    /// Scope role: retire every scope worker row (no warm recreate; next solve creates).
    /// Singleton role: kill + recreate pool slots as before.
    pub async fn force_rotate_proj(
        &self,
        proj_id: i64,
        slot_index: Option<u32>,
    ) -> Result<(), String> {
        let db = self.session_db().await?;
        let role = db
            .get_project_role(proj_id)
            .await
            .map_err(|e| format!("get_project_role: {e}"))?;
        if role == crate::master_observer::PROJECT_ROLE_SCOPE {
            // slot_index ignored — scope workers are keyed by scope_key, not pool slots.
            return self.force_retire_scope_workers(proj_id).await;
        }
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

    /// Kill + delete all scope workers for a project (Admin reset). Author: kejiqing
    async fn force_retire_scope_workers(&self, proj_id: i64) -> Result<(), String> {
        let db = self.session_db().await?;
        let rows = db
            .list_project_e2b_workers(proj_id)
            .await
            .map_err(|e| format!("list project_e2b_workers: {e}"))?;
        for existing in rows {
            if existing.scope_key.is_empty() {
                continue;
            }
            let slot_u = e2b_worker_slot_u32(existing.slot_index);
            let key = scope_slot_key(proj_id, &existing.scope_key, slot_u);
            let pg_busy = db
                .project_e2b_worker_is_busy_scoped(
                    proj_id,
                    &existing.scope_key,
                    existing.slot_index,
                )
                .await
                .map_err(|e| format!("project_e2b_worker_is_busy_scoped: {e}"))?;
            if self.active_leases(key.clone()).await > 0 || pg_busy {
                return Err(format!(
                    "proj_{proj_id} scope worker busy (scope_key present); wait for turns to finish"
                ));
            }
            info!(
                target: "claw_e2b_proj_worker",
                proj_id,
                scope_key = %existing.scope_key,
                sandbox_id = %existing.sandbox_id,
                "admin force retire scope worker"
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
                    reason: Some(format!(
                        "admin_force_reset;scope_key={}",
                        existing.scope_key
                    )),
                    at_ms: chrono::Utc::now().timestamp_millis(),
                },
            )
            .await;
            db.delete_project_e2b_worker_slot_scoped(
                proj_id,
                &existing.scope_key,
                existing.slot_index,
            )
            .await
            .map_err(|e| format!("delete project_e2b_worker scoped: {e}"))?;
            self.workers.lock().await.remove(&key);
            self.leases.lock().await.remove(&key);
        }
        Ok(())
    }

    async fn force_rotate_slot(&self, proj_id: i64, slot_index: u32) -> Result<(), String> {
        let db = self.session_db().await?;
        db.with_project_e2b_worker_slot_lock(proj_id, e2b_worker_slot_i32(slot_index), || async {
            self.force_rotate_slot_locked(proj_id, slot_index).await
        })
        .await
    }

    async fn force_rotate_slot_locked(&self, proj_id: i64, slot_index: u32) -> Result<(), String> {
        let db = self.session_db().await?;
        let spec = self.desired_worker_spec(proj_id).await?;
        let row = db
            .get_project_e2b_worker(proj_id, e2b_worker_slot_i32(slot_index))
            .await
            .map_err(|e| format!("get project_e2b_worker: {e}"))?;
        let key = singleton_slot_key(proj_id, slot_index);
        if let Some(ref existing) = row {
            let pg_busy = db
                .project_e2b_worker_is_busy(proj_id, e2b_worker_slot_i32(slot_index))
                .await
                .map_err(|e| format!("project_e2b_worker_is_busy: {e}"))?;
            if self.active_leases(key.clone()).await > 0 || pg_busy {
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
        self.create_and_persist_slot(proj_id, "", slot_index, &spec)
            .await?;
        Ok(())
    }

    pub async fn shutdown_all(&self) {
        self.workers.lock().await.clear();
        self.leases.lock().await.clear();
        self.pending_retire.lock().await.clear();
        info!(target: "claw_e2b_proj_worker", "shutdown_all (workers left running on e2b)");
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

    /// e2b alias for relaxed worker; PG may store `tpl_*` for the same template.
    const RELAXED_WORKER_ALIAS: &str = "claw-worker-relaxed";

    #[test]
    fn worker_contract_includes_project_home_rev_and_profile() {
        let key = worker_contract_key(
            "claw-worker-relaxed",
            None,
            "2026-07-01_12-00-00",
            "relaxed",
        );
        assert!(key.contains("nas-session-root-v3"));
        assert!(key.contains("#home=2026-07-01_12-00-00"));
        assert!(key.ends_with("#profile=relaxed"));
        assert!(
            !key.contains("env="),
            "env must not be part of worker contract"
        );
    }

    #[test]
    fn contract_with_build_id_roundtrip() {
        let key = worker_contract_key("tpl_a", Some("build-1"), "r", "strict");
        let parts = parse_worker_contract(&key).expect("parse");
        assert_eq!(parts.template, "tpl_a");
        assert_eq!(parts.build_id.as_deref(), Some("build-1"));
        assert_eq!(parts.home_rev, "r");
        assert_eq!(parts.profile, "strict");
        assert!(key.starts_with("tpl_a@build-1#"));
    }

    #[test]
    fn contract_without_build_id_legacy() {
        let legacy = worker_contract_key("tpl_a", None, "r", "strict");
        let parts = parse_worker_contract(&legacy).expect("parse");
        assert_eq!(parts.template, "tpl_a");
        assert!(parts.build_id.is_none());
        let with_build = worker_contract_key("tpl_a", Some("b2"), "r", "strict");
        // R1/legacy: runtime must not rotate solely because build is missing vs present.
        assert!(
            !needs_recreate(&legacy, &with_build, false, true),
            "remote rebuild must not rotate at runtime"
        );
    }

    #[test]
    fn alias_vs_tpl_relabel_no_kill() {
        let alias = worker_contract_key(RELAXED_WORKER_ALIAS, Some("b1"), "rev-1", "relaxed");
        let tpl = worker_contract_key("tpl_0153bc5c", Some("b1"), "rev-1", "relaxed");
        assert!(!contract_requires_rotation(&alias, &tpl));
        assert!(!contract_requires_rotation(&tpl, &alias));
        assert!(!needs_recreate(&alias, &tpl, false, true));
    }

    // R1: runtime buildId change must not recreate.
    #[test]
    fn r1_runtime_build_id_changed() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let b = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(
            !needs_recreate(&a, &b, false, true),
            "remote rebuild must not rotate at runtime"
        );
    }

    // R2: runtime tpl_* change must not recreate.
    #[test]
    fn r2_runtime_tpl_id_changed() {
        let a = worker_contract_key("tpl_aaaa", Some("b1"), "rev-1", "strict");
        let b = worker_contract_key("tpl_bbbb", Some("b1"), "rev-1", "strict");
        assert!(
            !needs_recreate(&a, &b, false, true),
            "tpl change must not rotate at runtime"
        );
        assert!(!contract_requires_rotation(&a, &b));
    }

    // R3: home_rev change still recreates.
    #[test]
    fn r3_runtime_home_rev_changed() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev-1", "strict");
        let b = worker_contract_key("tpl_a", Some("b1"), "rev-2", "strict");
        assert!(needs_recreate(&a, &b, false, true));
    }

    // R4: profile change still recreates.
    #[test]
    fn r4_runtime_profile_changed() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let b = worker_contract_key("tpl_a", Some("b1"), "rev", "relaxed");
        assert!(needs_recreate(&a, &b, false, true));
    }

    // R5: dead sandbox recreates even when contract matches.
    #[test]
    fn r5_runtime_dead_sandbox() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(needs_recreate(&a, &a, false, false));
    }

    // R6: same everything + alive → no recreate.
    #[test]
    fn r6_runtime_same_everything() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(!needs_recreate(&a, &a, false, true));
    }

    // S1: startup build mismatch → recreate.
    #[test]
    fn s1_startup_build_mismatch() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let b = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(needs_recreate(&a, &b, true, true));
    }

    // S2: startup same build → no recreate.
    #[test]
    fn s2_startup_build_same() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(!needs_recreate(&a, &a, true, true));
    }

    // S3: desired build empty → no force image refresh.
    #[test]
    fn s3_startup_desired_build_empty() {
        let stored = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let desired = worker_contract_key("tpl_a", None, "rev", "strict");
        assert!(
            !needs_recreate(&stored, &desired, true, true),
            "empty desired build must not force image refresh"
        );
    }

    // S4: legacy applied without @build, desired has build → recreate on startup.
    #[test]
    fn s4_startup_applied_legacy_no_build() {
        let legacy = worker_contract_key("tpl_a", None, "rev", "strict");
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(needs_recreate(&legacy, &desired, true, true));
    }

    #[test]
    fn protocol_version_mismatch_recreates() {
        let stored = "tpl_a@b1#nas-session-root-v3#home=rev#profile=strict";
        let desired = "tpl_a@b1#nas-session-root-v4#home=rev#profile=strict";
        assert!(needs_recreate(stored, desired, false, true));
    }

    #[test]
    fn relabel_keeps_applied_build_pin() {
        let stored = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let alias = worker_contract_key("claw-worker", Some("b1"), "rev", "strict");
        let newer = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(same_applied_build(&stored, &alias));
        assert!(!same_applied_build(&stored, &newer));
        assert!(!same_applied_build("not-a-contract", &newer));
    }

    // S5: dead even with same build → recreate.
    #[test]
    fn s5_startup_dead_even_same_build() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(needs_recreate(&a, &a, true, false));
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
