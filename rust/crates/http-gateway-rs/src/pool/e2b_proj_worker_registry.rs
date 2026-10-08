//! Per-project e2b worker registry — gateway-managed lifecycle (DB + e2b). Author: kejiqing
//!
//! Strict projects: N warm worker sandboxes per `proj_id` (global `e2bWorker.poolSize` default 1,
//! optional per-project `worker_profile_json.poolSize`, capped by `CLAW_E2B_POOL_SIZE_CAP`).
//! Relaxed: 1 worker (permissions-only; home mount rw). Full-pool reconcile on startup / Admin
//! poolSize change; solve acquire picks one slot from memory and reconciles only on cache miss.
//!
//! Image / buildId: remote rebuild only updates PG. Healthy sandboxes are never killed because
//! buildId/templateId changed at runtime. New image is applied via version switch
//! (`reconcile_version_switch` marks stale workers invalid) on **startup**, each **warm tick**,
//! and **acquire** entry (PG desired build may change without publish/restart). Manual reset or
//! dead/unhealthy sandboxes also rotate. Invalid workers are never acquired and leave the lease
//! ticker so they are not renewed.

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
use super::harness_engine;
use super::worker_profile::{
    default_worker_profile_json, effective_mode, load_desired_worker_pool_size, profile_mode_label,
    WorkerProfileMode,
};
use super::NasLayoutBackend;

const PROJECT_WORKER_CONTRACT_VERSION: &str = "nas-session-root-v3";
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

/// Build orphan-reap keep map from live PG workers (singleton + scope). Author: kejiqing
fn keep_by_proj_from_live_workers(rows: &[ProjectFcWorkerRow]) -> HashMap<i64, Vec<String>> {
    let mut keep_by_proj: HashMap<i64, Vec<String>> = HashMap::new();
    for row in rows {
        keep_by_proj
            .entry(row.proj_id)
            .or_default()
            .push(row.sandbox_id.clone());
    }
    keep_by_proj
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

/// Unified recreate decision for runtime warm/acquire: dead sandbox or contract mismatch.
/// BuildId alone never rotates at runtime — version switch marks stale workers `invalid`
/// (`reconcile_should_invalidate`) instead of recreating here. Author: kejiqing
fn needs_recreate(stored: &str, desired: &str, sandbox_alive: bool) -> bool {
    if !sandbox_alive {
        return true;
    }
    contract_requires_rotation(stored, desired)
}

/// Scope sleeping → wake: recreate when contract or build is behind desired.
///
/// Unlike singleton (`needs_recreate` never rotates on buildId), scope may catch up on build at
/// **resume** only — never while Running/busy. Author: kejiqing
#[must_use]
fn scope_wake_should_recreate(stored_contract: &str, desired_contract: &str) -> bool {
    contract_requires_rotation(stored_contract, desired_contract)
        || image_build_refresh_needed(stored_contract, desired_contract)
}

/// Reconcile (version switch) decision: does a stored contract need invalidation for the
/// desired build pin? Strictly `by_buildid` — a legacy row without a pin counts as stale.
/// Reconcile only marks `invalid`; it never creates a replacement. Author: kejiqing
#[must_use]
pub fn reconcile_should_invalidate(stored: &str, desired: &str) -> bool {
    image_build_refresh_needed(stored, desired)
}

/// Acquire guard: slot may be leased only when lifecycle is not `invalid` and applied build
/// is not behind desired (same rule as version switch). Author: kejiqing
#[must_use]
pub fn acquire_slot_usable(
    lifecycle: &str,
    stored_contract: &str,
    desired_contract: &str,
) -> bool {
    if lifecycle == "invalid" {
        return false;
    }
    !reconcile_should_invalidate(stored_contract, desired_contract)
}

/// A persisted singleton worker's availability for acquire + warm accounting.
/// Single source of truth for "can this worker be handed to a solve / counted toward warm".
/// Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SingletonWorkerState {
    /// running + alive: usable for acquire, counts toward warm fullness.
    Usable,
    /// marked invalid (version switch): never acquired, never counted toward warm.
    Invalid,
    /// idle-paused: needs resume before use, not counted toward warm.
    Sleeping,
    /// sandbox dead / missing: must be rebuilt.
    Dead,
}

/// Classify a singleton worker from `lifecycle_state` + live probe. Author: kejiqing
#[must_use]
pub fn classify_singleton_worker(
    lifecycle_state: &str,
    sandbox_alive: bool,
) -> SingletonWorkerState {
    match lifecycle_state {
        "invalid" => SingletonWorkerState::Invalid,
        "sleeping" => SingletonWorkerState::Sleeping,
        "running" if sandbox_alive => SingletonWorkerState::Usable,
        _ => SingletonWorkerState::Dead,
    }
}

/// How many additional singleton workers warm must create to reach `pool_size` running+alive.
/// `invalid`/`sleeping`/dead slots never count toward fullness (they are not usable). Author: kejiqing
#[must_use]
pub fn warm_singleton_shortfall(pool_size: u32, states: &[SingletonWorkerState]) -> u32 {
    let usable = states
        .iter()
        .filter(|s| **s == SingletonWorkerState::Usable)
        .count() as u32;
    pool_size.saturating_sub(usable)
}

/// Per-worker action warm decides for one tick. This is the decision layer — the async
/// executor only carries these out, so the whole matrix is unit-testable without a live e2b
/// client or PG. Author: kejiqing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmWorkerAction {
    /// running+alive: renew TTL (keep, counts toward fullness).
    Renew,
    /// running but sandbox dead: retire (kill) + delete row (shortfall recreates).
    KillAndDelete,
    /// invalid and sandbox gone: reap the PG row (no kill — already dead).
    ReapInvalid,
    /// invalid but sandbox alive (in-flight request), or sleeping: leave alone.
    Leave,
}

/// Plan warm actions for a project's singleton workers from `(lifecycle_state, alive)` pairs,
/// returning per-worker actions (parallel to input) plus the create shortfall.
/// Author: kejiqing
#[must_use]
pub fn plan_warm_actions(
    pool_size: u32,
    workers: &[(String, bool)],
) -> (Vec<WarmWorkerAction>, u32) {
    let mut actions = Vec::with_capacity(workers.len());
    let mut states = Vec::with_capacity(workers.len());
    for (state, alive) in workers {
        let classified = classify_singleton_worker(state, *alive);
        states.push(classified);
        match classified {
            SingletonWorkerState::Usable => actions.push(WarmWorkerAction::Renew),
            SingletonWorkerState::Dead => actions.push(WarmWorkerAction::KillAndDelete),
            SingletonWorkerState::Invalid => {
                if *alive {
                    actions.push(WarmWorkerAction::Leave);
                } else {
                    actions.push(WarmWorkerAction::ReapInvalid);
                }
            }
            SingletonWorkerState::Sleeping => actions.push(WarmWorkerAction::Leave),
        }
    }
    (actions, warm_singleton_shortfall(pool_size, &states))
}

/// Indices (into `rows`) of workers that reconcile must mark invalid for `desired_contract`
/// (version switch). Never includes already-invalid rows; never returns a kill action — reconcile
/// only marks invalid, warm fills the pool. Author: kejiqing
#[must_use]
pub fn plan_reconcile_invalidations(
    desired_contract: &str,
    rows: &[ProjectFcWorkerRow],
) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| r.lifecycle_state != "invalid")
        .filter(|(_, r)| reconcile_should_invalidate(&r.template_id, desired_contract))
        .map(|(i, _)| i)
        .collect()
}

/// Smallest non-negative slot index not present in `used`. Author: kejiqing
#[must_use]
pub fn next_free_slot(used: &std::collections::HashSet<u32>) -> u32 {
    let mut slot = 0u32;
    while used.contains(&slot) {
        slot += 1;
    }
    slot
}

struct WorkerSpec {
    e2b_template_id: String,
    build_id: Option<String>,
    mode: WorkerProfileMode,
    profile_label: String,
    /// Template alias used as create target (buildId pinned separately).
    create_alias: String,
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
        let engine = harness_engine::load_project_harness_engine(db.as_ref(), proj_id).await?;
        if let Some(t) = harness_engine::engine_worker_template(db.as_ref(), engine).await? {
            return Ok(WorkerSpec {
                e2b_template_id: t.template_id,
                build_id: t.build_id,
                mode: WorkerProfileMode::Strict,
                profile_label: t.profile_label,
                create_alias: t.alias,
            });
        }
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
                    create_alias: e2b_worker_relaxed_template_from_env(),
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
                    create_alias: e2b_worker_template_from_env(),
                })
            }
        }
    }

    /// Desired worker contract for proj (same source as create/reconcile). Author: kejiqing
    async fn desired_contract_for_proj(&self, proj_id: i64) -> Result<String, String> {
        let spec = self.desired_worker_spec(proj_id).await?;
        let db = self.session_db().await?;
        desired_worker_contract(
            db.as_ref(),
            &spec.e2b_template_id,
            spec.build_id.as_deref(),
            proj_id,
            &spec.profile_label,
        )
        .await
    }

    async fn desired_pool_size(&self, proj_id: i64) -> Result<u32, String> {
        let db = self.session_db().await?;
        load_desired_worker_pool_size(db.as_ref(), proj_id).await
    }

    /// Version switch: mark workers on an older buildId `invalid`.
    ///
    /// Invoked on startup, each warm tick, and acquire entry — not gated on publish completion.
    /// - Never kills (in-flight requests keep running); never creates a replacement (warm does).
    /// - Atomic: desired buildId is read once per project and all stale workers are marked in
    ///   the same pass — warm only ever reads the new buildId after this returns.
    /// - Applies to singleton **and** scope rows; scope rebuild is resolve-driven (Phase E).
    ///
    /// Author: kejiqing
    pub async fn reconcile_version_switch(&self) -> Result<(), String> {
        let db = self.session_db().await?;
        let proj_ids = db
            .list_project_config_proj_ids()
            .await
            .map_err(|e| format!("list project_config proj_ids: {e}"))?;
        info!(
            target: "claw_e2b_proj_worker",
            proj_count = proj_ids.len(),
            "reconcile version switch (mark stale buildId invalid)"
        );
        let mut errors = Vec::new();
        for proj_id in proj_ids {
            if let Err(e) = self.invalidate_stale_build_for_proj(proj_id).await {
                errors.push(format!("proj {proj_id}: {e}"));
            }
        }
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        // Re-register persisted sandboxes with the lease ticker after a gateway restart
        // (in-memory tracking is lost; warm ticker renews later but this seeds immediately).
        self.seed_lease_tracking_from_db().await;
        Ok(())
    }

    /// Mark every worker (singleton + scope) whose applied buildId differs from desired as
    /// `invalid`. Cache entries are dropped so acquire can never hand an invalid worker out.
    /// Author: kejiqing
    async fn invalidate_stale_build_for_proj(&self, proj_id: i64) -> Result<(), String> {
        let db = self.session_db().await?;
        let desired = self.desired_contract_for_proj(proj_id).await?;
        let rows = db
            .list_project_e2b_workers(proj_id)
            .await
            .map_err(|e| format!("list_project_e2b_workers: {e}"))?;
        for idx in plan_reconcile_invalidations(&desired, &rows) {
            let row = &rows[idx];
            db.invalidate_project_e2b_worker_slot_scoped(
                proj_id,
                &row.scope_key,
                row.slot_index,
                "version_switch",
            )
            .await
            .map_err(|e| format!("invalidate slot {}: {e}", row.slot_index))?;
            let key = scope_slot_key(proj_id, &row.scope_key, e2b_worker_slot_u32(row.slot_index));
            self.workers.lock().await.remove(&key);
            // Stop lease ticker so invalid sandboxes are not renewed. Author: kejiqing
            self.client
                .unregister_tracked_sandbox(&row.sandbox_id);
            audit_rotation(
                db.as_ref(),
                WorkerRotationEvent {
                    proj_id,
                    event: "invalidated".to_string(),
                    sandbox_id: Some(row.sandbox_id.clone()),
                    worker_id: Some(row.worker_id.clone()),
                    template_id: Some(row.template_id.clone()),
                    reason: Some("version_switch".to_string()),
                    at_ms: chrono::Utc::now().timestamp_millis(),
                },
            )
            .await;
            info!(
                target: "claw_e2b_proj_worker",
                proj_id,
                scope_key = %row.scope_key,
                slot_index = row.slot_index,
                sandbox_id = %row.sandbox_id,
                "worker marked invalid for version switch (no kill)"
            );
        }
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
        // Keep **all** live workers (singleton + scope). Scope sandboxes also carry
        // clawRole=warm-proj; singleton-only keep lists kill them on gateway restart/release.
        // Author: kejiqing
        let mut keep_by_proj: HashMap<i64, Vec<String>> = HashMap::new();
        if let Ok(rows) = db.list_cluster_e2b_live_workers().await {
            keep_by_proj = keep_by_proj_from_live_workers(&rows);
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

    /// Full-pool reconcile for one project (Admin poolSize change): build/fill slots 0..pool_size
    /// and retire overflow. Runtime path — never rotates on buildId (version switch is
    /// `reconcile_version_switch`). Author: kejiqing
    pub async fn reconcile_proj(&self, proj_id: i64) -> Result<(), String> {
        let pool_size = self.desired_pool_size(proj_id).await?;
        for slot_index in 0..pool_size {
            self.reconcile_proj_slot(proj_id, slot_index).await?;
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

    async fn reconcile_proj_slot(&self, proj_id: i64, slot_index: u32) -> Result<(), String> {
        let db = self.session_db().await?;
        db.with_project_e2b_worker_slot_lock(proj_id, e2b_worker_slot_i32(slot_index), || async {
            self.reconcile_proj_slot_locked(proj_id, slot_index).await
        })
        .await
    }

    async fn reconcile_proj_slot_locked(
        &self,
        proj_id: i64,
        slot_index: u32,
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
            let must_recreate =
                needs_recreate(&existing.template_id, &desired_contract, sandbox_alive);
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
                "proj worker rotate (contract / offline)"
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
            // Keep live singleton + scope workers for this proj (same clawRole=warm-proj).
            // Author: kejiqing
            let keep: Vec<String> = db
                .list_project_e2b_live_workers(proj_id)
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
        let template_ref = claw_e2b_sandbox_client::e2b_sandbox_template_ref(
            &spec.create_alias,
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

        // Project preflight worker.init.* before slot is ready (pre-Landlock). Author: kejiqing
        let solve_preflight_json = db
            .get_project_config(proj_id)
            .await
            .map(|row| row.map(|r| r.solve_preflight_json))
            .map_err(|e| format!("load solve_preflight_json for proj {proj_id}: {e}"))?
            .unwrap_or_else(|| json!({"kind": "none"}));
        let plugin_defaults = db
            .list_preflight_plugins()
            .await
            .map_err(|e| format!("load preflight plugin catalog for proj {proj_id}: {e}"))?;
        let init_mode = match spec.mode {
            WorkerProfileMode::Relaxed => "relaxed",
            WorkerProfileMode::Strict => "strict",
        };
        if let Err(e) = super::worker_lifecycle_preflight::run_worker_init_on_create(
            &self.client,
            &handle,
            &solve_preflight_json,
            &plugin_defaults,
            proj_id,
            &worker_id,
            &contract_key,
            init_mode,
        )
        .await
        {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!(
                "worker.init preflight failed for proj {proj_id} worker {worker_id}: {e}"
            ));
        }

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
        self.reconcile_proj_slot(proj_id, 0).await?;
        let key = singleton_slot_key(proj_id, 0);
        let guard = self.workers.lock().await;
        let rt = guard
            .get(&key)
            .ok_or_else(|| format!("proj worker missing after reconcile proj_{proj_id} slot 0"))?;
        Ok((rt.handle.clone(), rt.worker_id.clone()))
    }

    /// Strict solve: pick a usable singleton slot (least lease) and acquire it.
    ///
    /// Hot path: version-switch invalidate → `pick_usable_singleton_slot` (cache) →
    /// `acquire_slot`; falls back to creating a fresh slot when no warm worker exists.
    /// Author: kejiqing
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
        // PG desired build may advance without restart/publish — mark stale before pick.
        self.invalidate_stale_build_for_proj(proj_id).await?;
        // Pick a usable (running non-invalid) slot from cache; fall back to creating a fresh
        // slot (warm keeps the pool full — this is only a solve-time safety net). Author: kejiqing
        let slot_index = match self.pick_usable_singleton_slot(proj_id).await? {
            Some(slot) => slot,
            None => self.next_free_singleton_slot(proj_id).await?,
        };
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
        // PG desired build may advance without restart/publish — mark stale before lease.
        self.invalidate_stale_build_for_proj(proj_id).await?;
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
                    // Sleeping → wake: catch up image/contract before resume (busy path untouched).
                    // Author: kejiqing
                    let desired = self.desired_contract_for_proj(proj_id).await?;
                    if scope_wake_should_recreate(&existing.template_id, &desired) {
                        self.invalidate_dead_scope_worker(
                            proj_id,
                            scope_key,
                            0,
                            &existing.sandbox_id,
                            &existing.worker_id,
                            &existing.template_id,
                            "scope_wake_image_or_contract",
                        )
                        .await?;
                    } else {
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

    /// Pick a usable (running non-invalid) singleton slot from cache: least lease + tie-break.
    /// Invalid/dead workers are never in cache (invalidated removes them; dead are reconciled on
    /// acquire), so cache membership is the "usable" filter. Author: kejiqing
    async fn pick_usable_singleton_slot(&self, proj_id: i64) -> Result<Option<u32>, String> {
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
        self.reconcile_proj_slot(proj_id, slot_index).await
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
                // Cache-hit resume: same wake gate as acquire (image/contract catch-up).
                // Author: kejiqing
                let (worker_id, template_id) = self
                    .scope_worker_ids_from_cache_or_db(proj_id, scope_key, slot_index)
                    .await;
                let desired = self.desired_contract_for_proj(proj_id).await?;
                if scope_wake_should_recreate(&template_id, &desired) {
                    self.invalidate_dead_scope_worker(
                        proj_id,
                        scope_key,
                        slot_index,
                        sandbox_id,
                        &worker_id,
                        &template_id,
                        "scope_wake_image_or_contract",
                    )
                    .await?;
                    return Ok(ScopeWorkerReady::Dropped);
                }
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
        let warm_sandbox_id = {
            let guard = self.workers.lock().await;
            guard.get(&key).map(|rt| rt.handle.sandbox_id.clone())
        };
        let mut warm_hit = false;
        if let Some(ref sandbox_id) = warm_sandbox_id {
            self.ensure_warm_worker_running(proj_id, slot_index, sandbox_id)
                .await?;
            // Defense: refuse invalid / build-behind even if still in cache. Author: kejiqing
            let desired = self.desired_contract_for_proj(proj_id).await?;
            let db = self.session_db().await?;
            let row = db
                .get_project_e2b_worker(proj_id, e2b_worker_slot_i32(slot_index))
                .await
                .map_err(|e| format!("get project_e2b_worker for acquire guard: {e}"))?;
            let usable = match &row {
                Some(r) => {
                    acquire_slot_usable(&r.lifecycle_state, &r.template_id, &desired)
                }
                // get_* filters out invalid — treat missing as unusable for this warm hit.
                None => false,
            };
            if usable {
                warm_hit = true;
            } else {
                if let Some(r) = &row {
                    let _ = db
                        .invalidate_project_e2b_worker_slot_scoped(
                            proj_id,
                            "",
                            e2b_worker_slot_i32(slot_index),
                            "acquire_guard",
                        )
                        .await;
                    self.client.unregister_tracked_sandbox(&r.sandbox_id);
                } else {
                    self.client.unregister_tracked_sandbox(sandbox_id);
                }
                self.workers.lock().await.remove(&key);
                info!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    slot_index,
                    sandbox_id = %sandbox_id,
                    "acquire skipped unusable warm slot (invalid or stale build)"
                );
            }
        }
        if warm_hit {
            let guard = self.workers.lock().await;
            let rt = guard.get(&key).ok_or_else(|| {
                format!("proj worker missing after warm verify proj_{proj_id} slot {slot_index}")
            })?;
            let handle = rt.handle.clone();
            let worker_id = rt.worker_id.clone();
            let template_id = rt.template_id.clone();
            drop(guard);
            // Existing warm worker: worker.reuse.start (not create). Author: kejiqing
            if let Ok(db) = self.session_db().await {
                let solve_preflight_json = db
                    .get_project_config(proj_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|r| r.solve_preflight_json)
                    .unwrap_or_else(|| json!({"kind": "none"}));
                let plugin_defaults = db.list_preflight_plugins().await.unwrap_or_default();
                let profile = db
                    .get_worker_profile_json(proj_id)
                    .await
                    .unwrap_or_else(|_| default_worker_profile_json());
                let mode = profile_mode_label(&profile);
                if let Err(e) =
                    super::worker_lifecycle_preflight::run_worker_reuse_start_on_acquire(
                        &self.client,
                        &handle,
                        &solve_preflight_json,
                        &plugin_defaults,
                        proj_id,
                        &worker_id,
                        &template_id,
                        &handle.sandbox_id,
                        mode,
                        None,
                    )
                    .await
                {
                    return Err(format!(
                        "worker.reuse.start preflight failed for proj {proj_id}: {e}"
                    ));
                }
            }
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
        // Runtime path: buildId alone never rotates (version switch marks invalid). Author: kejiqing
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

    /// Warm ticker: version-switch stale builds, then keep every non-scope project at
    /// `pool_size` usable singleton workers (probe, rebuild dead, fill shortfall, renew TTL).
    /// Author: kejiqing
    pub fn spawn_warm_ticker(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(self.renew_interval_secs));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(e) = self.warm_reconcile_once().await {
                    warn!(
                        target: "claw_e2b_proj_worker",
                        error = %e,
                        "warm ticker reconcile failed (best-effort)"
                    );
                }
            }
        });
    }

    /// One warm pass: mark stale builds invalid first (PG may advance without publish), then
    /// fill pools. Best-effort; never fails the process. Author: kejiqing
    async fn warm_reconcile_once(&self) -> Result<(), String> {
        if let Err(e) = self.reconcile_version_switch().await {
            warn!(
                target: "claw_e2b_proj_worker",
                error = %e,
                "warm tick version switch failed (best-effort)"
            );
        }
        let db = self.session_db().await?;
        let proj_ids = db
            .list_project_config_proj_ids()
            .await
            .map_err(|e| format!("list project_config proj_ids: {e}"))?;
        for proj_id in proj_ids {
            if let Err(e) = self.warm_reconcile_proj(proj_id).await {
                warn!(
                    target: "claw_e2b_proj_worker",
                    proj_id,
                    error = %e,
                    "warm reconcile proj failed (best-effort)"
                );
            }
        }
        self.reap_cluster_warm_proj_orphans_best_effort().await;
        Ok(())
    }

    /// Warm one project to `pool_size` usable singleton workers. Scope roles are skipped
    /// (slots=0 → warm idle; resolve drives their lifecycle). Author: kejiqing
    async fn warm_reconcile_proj(&self, proj_id: i64) -> Result<(), String> {
        let db = self.session_db().await?;
        let role = db
            .get_project_role(proj_id)
            .await
            .map_err(|e| format!("get_project_role: {e}"))?;
        if role == crate::master_observer::PROJECT_ROLE_SCOPE {
            return Ok(()); // S1: scope has no warm pool
        }
        let pool_size = self.desired_pool_size(proj_id).await?;
        let spec = self.desired_worker_spec(proj_id).await?;
        let rows = db
            .list_project_e2b_workers(proj_id)
            .await
            .map_err(|e| format!("list_project_e2b_workers: {e}"))?;
        let singleton: Vec<&ProjectFcWorkerRow> =
            rows.iter().filter(|r| r.scope_key.is_empty()).collect();

        // Probe liveness once, then let the pure planner decide actions + shortfall.
        let mut probes: Vec<(String, bool)> = Vec::with_capacity(singleton.len());
        for row in &singleton {
            let alive = self.client.sandbox_running(&row.sandbox_id).await
                || self.client.sandbox_paused(&row.sandbox_id).await;
            probes.push((row.lifecycle_state.clone(), alive));
        }
        let (actions, shortfall) = plan_warm_actions(pool_size, &probes);

        for (row, action) in singleton.iter().zip(actions) {
            match action {
                WarmWorkerAction::Renew => {
                    if let Err(e) = self
                        .client
                        .renew_sandbox_ttl_secs(&row.sandbox_id, self.worker_ttl_secs)
                        .await
                    {
                        warn!(
                            target: "claw_e2b_proj_worker",
                            proj_id,
                            sandbox_id = %row.sandbox_id,
                            error = %e,
                            "warm TTL renew failed"
                        );
                    }
                }
                WarmWorkerAction::KillAndDelete => {
                    self.retire_worker_sandbox(proj_id, &row.sandbox_id).await;
                    db.delete_project_e2b_worker_slot(proj_id, row.slot_index)
                        .await
                        .map_err(|e| format!("delete dead slot {}: {e}", row.slot_index))?;
                }
                WarmWorkerAction::ReapInvalid => {
                    db.delete_project_e2b_worker_slot(proj_id, row.slot_index)
                        .await
                        .map_err(|e| format!("reap invalid slot {}: {e}", row.slot_index))?;
                }
                WarmWorkerAction::Leave => {}
            }
        }

        // Fill shortfall with fresh slots (invalid occupies its slot; slots may transiently
        // exceed pool_size until the invalid row is reaped).
        for _ in 0..shortfall {
            let slot = self.next_free_singleton_slot(proj_id).await?;
            self.create_and_persist_slot(proj_id, "", slot, &spec)
                .await?;
        }
        Ok(())
    }

    /// Smallest free singleton slot index (skips invalid/dead rows still occupying theirs).
    /// Author: kejiqing
    async fn next_free_singleton_slot(&self, proj_id: i64) -> Result<u32, String> {
        let db = self.session_db().await?;
        let rows = db
            .list_project_e2b_workers(proj_id)
            .await
            .map_err(|e| format!("list_project_e2b_workers: {e}"))?;
        let used: std::collections::HashSet<u32> = rows
            .iter()
            .filter(|r| r.scope_key.is_empty())
            .map(|r| e2b_worker_slot_u32(r.slot_index))
            .collect();
        Ok(next_free_slot(&used))
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

/// Pure least-lease slot picker over candidate (usable) slots.
///
/// Candidates are the running non-invalid singleton slots already present in cache — acquire
/// never picks an invalid/dead slot (those are filtered out by callers before reaching here).
/// Returns `None` when no candidate exists (caller creates a fresh slot). Author: kejiqing
fn select_least_lease_slot(
    present_slots: &[u32],
    lease_by_slot: &HashMap<u32, u32>,
    tie_break: u32,
) -> Option<u32> {
    let mut present: Vec<u32> = present_slots.to_vec();
    present.sort_unstable();
    present.dedup();
    if present.is_empty() {
        return None;
    }
    let mut best_slot = present[0];
    let mut best_count = lease_by_slot.get(&best_slot).copied().unwrap_or(0);
    for &slot in &present[1..] {
        let count = lease_by_slot.get(&slot).copied().unwrap_or(0);
        if count < best_count {
            best_count = count;
            best_slot = slot;
        }
    }
    // Deterministic tie-break: rotate among candidates by `tie_break`.
    let idx = present.iter().position(|&s| s == best_slot).unwrap_or(0);
    let n = present.len();
    Some(present[(idx + (tie_break as usize % n)) % n])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway_e2b_lifecycle_decision::ScopeSandboxProbe;

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
            !needs_recreate(&legacy, &with_build, true),
            "remote rebuild must not rotate at runtime"
        );
    }

    #[test]
    fn alias_vs_tpl_relabel_no_kill() {
        let alias = worker_contract_key(RELAXED_WORKER_ALIAS, Some("b1"), "rev-1", "relaxed");
        let tpl = worker_contract_key("tpl_0153bc5c", Some("b1"), "rev-1", "relaxed");
        assert!(!contract_requires_rotation(&alias, &tpl));
        assert!(!contract_requires_rotation(&tpl, &alias));
        assert!(!needs_recreate(&alias, &tpl, true));
    }

    // R1: runtime buildId change must not recreate.
    #[test]
    fn r1_runtime_build_id_changed() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let b = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(
            !needs_recreate(&a, &b, true),
            "remote rebuild must not rotate at runtime"
        );
    }

    // R2: runtime tpl_* change must not recreate.
    #[test]
    fn r2_runtime_tpl_id_changed() {
        let a = worker_contract_key("tpl_aaaa", Some("b1"), "rev-1", "strict");
        let b = worker_contract_key("tpl_bbbb", Some("b1"), "rev-1", "strict");
        assert!(
            !needs_recreate(&a, &b, true),
            "tpl change must not rotate at runtime"
        );
        assert!(!contract_requires_rotation(&a, &b));
    }

    // R3: home_rev change still recreates.
    #[test]
    fn r3_runtime_home_rev_changed() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev-1", "strict");
        let b = worker_contract_key("tpl_a", Some("b1"), "rev-2", "strict");
        assert!(needs_recreate(&a, &b, true));
    }

    // R4: profile change still recreates.
    #[test]
    fn r4_runtime_profile_changed() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let b = worker_contract_key("tpl_a", Some("b1"), "rev", "relaxed");
        assert!(needs_recreate(&a, &b, true));
    }

    // R5: dead sandbox recreates even when contract matches.
    #[test]
    fn r5_runtime_dead_sandbox() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(needs_recreate(&a, &a, false));
    }

    // R6: same everything + alive → no recreate.
    #[test]
    fn r6_runtime_same_everything() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(!needs_recreate(&a, &a, true));
    }

    // S1: version switch marks stale buildId invalid (no recreate in place).
    #[test]
    fn s1_version_switch_build_mismatch_invalidates() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let b = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(reconcile_should_invalidate(&a, &b));
    }

    // S2: version switch same build → no invalidate.
    #[test]
    fn s2_version_switch_build_same_no_invalidate() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(!reconcile_should_invalidate(&a, &a));
    }

    // S3: desired build empty → no version switch.
    #[test]
    fn s3_version_switch_desired_build_empty() {
        let stored = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let desired = worker_contract_key("tpl_a", None, "rev", "strict");
        assert!(!reconcile_should_invalidate(&stored, &desired));
    }

    // S4: legacy applied without @build, desired has build → invalidate (stale).
    #[test]
    fn s4_version_switch_legacy_no_build_invalidates() {
        let legacy = worker_contract_key("tpl_a", None, "rev", "strict");
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(reconcile_should_invalidate(&legacy, &desired));
    }

    #[test]
    fn protocol_version_mismatch_recreates() {
        let stored = "tpl_a@b1#nas-session-root-v3#home=rev#profile=strict";
        let desired = "tpl_a@b1#nas-session-root-v4#home=rev#profile=strict";
        assert!(needs_recreate(stored, desired, true));
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
    fn s5_runtime_dead_even_same_build() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(needs_recreate(&a, &a, false));
    }

    #[test]
    fn least_lease_picks_lowest_lease_count() {
        let present = vec![0, 1, 2, 3];
        let leases = HashMap::from([(0, 2), (1, 0), (2, 1), (3, 3)]);
        assert_eq!(select_least_lease_slot(&present, &leases, 0), Some(1));
    }

    #[test]
    fn least_lease_tie_break_is_deterministic() {
        let present = vec![0, 1];
        let leases = HashMap::from([(0, 0), (1, 0)]);
        assert_eq!(select_least_lease_slot(&present, &leases, 0), Some(0));
        assert_eq!(select_least_lease_slot(&present, &leases, 1), Some(1));
    }

    #[test]
    fn least_lease_returns_none_when_no_candidate() {
        let present: Vec<u32> = vec![];
        let leases = HashMap::new();
        assert_eq!(select_least_lease_slot(&present, &leases, 0), None);
    }

    #[test]
    fn least_lease_skips_invalid_slot_not_in_candidates() {
        // invalid slot 0 is not a candidate (removed from cache); acquire picks among the rest.
        let present = vec![1, 2];
        let leases = HashMap::from([(1, 2), (2, 0)]);
        assert_eq!(select_least_lease_slot(&present, &leases, 0), Some(2));
    }

    fn live_row(proj_id: i64, scope_key: &str, sandbox_id: &str) -> ProjectFcWorkerRow {
        ProjectFcWorkerRow {
            proj_id,
            scope_key: scope_key.to_string(),
            slot_index: 0,
            sandbox_id: sandbox_id.to_string(),
            worker_id: "w".into(),
            template_id: "tpl".into(),
            handle_json: json!({}),
            updated_at_ms: 0,
            in_use_count: 0,
            in_use_until_ms: 0,
            lifecycle_state: "running".into(),
            last_idle_at_ms: 0,
            mcp_bind_json: json!({}),
            invalid_reason: String::new(),
        }
    }

    /// Regression: release/startup orphan reap must keep scope workers, not only singleton.
    #[test]
    fn startup_orphan_keep_includes_scope_workers() {
        let rows = vec![
            live_row(3024, "", "sbx-singleton"),
            live_row(3024, "fda-role", "sbx-scope-fda"),
            live_row(1001, "", "sbx-other"),
        ];
        let keep = keep_by_proj_from_live_workers(&rows);
        let ids = keep.get(&3024).expect("proj 3024 keep");
        assert!(ids.contains(&"sbx-singleton".to_string()));
        assert!(
            ids.contains(&"sbx-scope-fda".to_string()),
            "scope sandbox must be in keep or release restart kills it"
        );
        assert_eq!(
            keep.get(&1001).map(Vec::as_slice),
            Some(["sbx-other".to_string()].as_slice())
        );
        assert!(claw_e2b_sandbox_client::warm_proj_sandbox_kept(
            3024,
            "sbx-scope-fda",
            &keep
        ));
    }

    #[test]
    fn scope_wake_recreates_on_build_mismatch() {
        let old = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let new = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(scope_wake_should_recreate(&old, &new));
        // Singleton never rotates on buildId alone (version switch invalidates instead):
        assert!(!needs_recreate(&old, &new, true));
        assert!(reconcile_should_invalidate(&old, &new));
    }

    #[test]
    fn scope_wake_resumes_when_build_and_contract_match() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(!scope_wake_should_recreate(&a, &a));
    }

    #[test]
    fn scope_wake_recreates_on_home_rev_change() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev-1", "strict");
        let b = worker_contract_key("tpl_a", Some("b1"), "rev-2", "strict");
        assert!(scope_wake_should_recreate(&a, &b));
    }

    #[test]
    fn scope_wake_recreates_on_profile_change() {
        let a = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let b = worker_contract_key("tpl_a", Some("b1"), "rev", "relaxed");
        assert!(scope_wake_should_recreate(&a, &b));
    }

    #[test]
    fn scope_wake_recreates_legacy_row_without_build_pin() {
        let legacy = worker_contract_key("tpl_a", None, "rev", "strict");
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(scope_wake_should_recreate(&legacy, &desired));
    }

    #[test]
    fn scope_wake_no_image_recreate_when_desired_build_empty() {
        let stored = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let desired_no_pin = worker_contract_key("tpl_a", None, "rev", "strict");
        assert!(
            !scope_wake_should_recreate(&stored, &desired_no_pin),
            "empty desired build pin must not force image catch-up on wake"
        );
        assert!(!image_build_refresh_needed(&stored, &desired_no_pin));
    }

    /// Invariant: wake contract check is only entered on Resume, never on Reuse (busy/running).
    #[test]
    fn scope_wake_gate_only_applies_when_lifecycle_says_resume() {
        let stale = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(
            scope_wake_should_recreate(&stale, &desired),
            "contracts diverge — would recreate IF on Resume path"
        );
        let reuse = decide_scope_existing_worker("running", ScopeSandboxProbe::Running);
        assert_eq!(reuse, ScopeWorkerAction::Reuse);
        assert!(
            !matches!(reuse, ScopeWorkerAction::Resume),
            "Running must not enter wake gate (caller skips scope_wake_should_recreate)"
        );
        let resume = decide_scope_existing_worker("sleeping", ScopeSandboxProbe::Paused);
        assert_eq!(resume, ScopeWorkerAction::Resume);
    }

    // ---- reconcile (version switch): by_buildid, no kill ----

    #[test]
    fn reconcile_invalidates_only_on_build_change() {
        let same = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(
            !reconcile_should_invalidate(&same, &same),
            "R1: same build must not invalidate"
        );
        let newer = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(
            reconcile_should_invalidate(&same, &newer),
            "R2: build changed must invalidate"
        );
    }

    #[test]
    fn reconcile_invalidates_legacy_row_without_pin() {
        let legacy = worker_contract_key("tpl_a", None, "rev", "strict");
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(
            reconcile_should_invalidate(&legacy, &desired),
            "R5: legacy no-pin is stale"
        );
    }

    #[test]
    fn reconcile_ignores_empty_desired_pin() {
        let stored = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let no_pin = worker_contract_key("tpl_a", None, "rev", "strict");
        assert!(!reconcile_should_invalidate(&stored, &no_pin));
    }

    // ---- classify singleton worker (acquire + warm accounting) ----

    #[test]
    fn classify_singleton_running_alive_is_usable() {
        assert_eq!(
            classify_singleton_worker("running", true),
            SingletonWorkerState::Usable
        );
    }

    #[test]
    fn classify_singleton_invalid_is_never_usable() {
        // N1/N2: invalid never usable even when sandbox still alive.
        assert_eq!(
            classify_singleton_worker("invalid", true),
            SingletonWorkerState::Invalid
        );
        assert_eq!(
            classify_singleton_worker("invalid", false),
            SingletonWorkerState::Invalid
        );
    }

    #[test]
    fn classify_singleton_sleeping_needs_resume() {
        assert_eq!(
            classify_singleton_worker("sleeping", true),
            SingletonWorkerState::Sleeping
        );
    }

    #[test]
    fn classify_singleton_dead_sandbox_is_dead() {
        assert_eq!(
            classify_singleton_worker("running", false),
            SingletonWorkerState::Dead
        );
    }

    // ---- warm fullness ----

    #[test]
    fn warm_shortfall_zero_when_full() {
        // W1: usable == poolSize → no create.
        let states = [SingletonWorkerState::Usable, SingletonWorkerState::Usable];
        assert_eq!(warm_singleton_shortfall(2, &states), 0);
    }

    #[test]
    fn warm_shortfall_counts_missing() {
        // W2: usable < poolSize → create the gap.
        let states = [SingletonWorkerState::Usable];
        assert_eq!(warm_singleton_shortfall(3, &states), 2);
    }

    #[test]
    fn warm_shortfall_excludes_invalid_and_dead() {
        // W4: invalid occupies a slot but never counts; dead never counts.
        let states = [
            SingletonWorkerState::Usable,
            SingletonWorkerState::Invalid,
            SingletonWorkerState::Dead,
        ];
        assert_eq!(warm_singleton_shortfall(3, &states), 2);
    }

    #[test]
    fn warm_shortfall_saturates_at_zero() {
        // More usable than pool_size (transient extra slot) never underflows.
        let states = [SingletonWorkerState::Usable, SingletonWorkerState::Usable];
        assert_eq!(warm_singleton_shortfall(1, &states), 0);
    }

    // ---- warm action planning (decision layer) ----

    fn st(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn plan_warm_renews_usable_only() {
        // Constraint 3 / W6: invalid must not Renew (Leave); only running+alive renew.
        let (actions, shortfall) = plan_warm_actions(
            2,
            &[
                (st("running"), true),
                (st("running"), false),
                (st("invalid"), true),
                (st("sleeping"), true),
            ],
        );
        assert_eq!(
            actions,
            vec![
                WarmWorkerAction::Renew,
                WarmWorkerAction::KillAndDelete,
                WarmWorkerAction::Leave,
                WarmWorkerAction::Leave,
            ]
        );
        // Only 1 usable → shortfall 1.
        assert_eq!(shortfall, 1);
    }

    #[test]
    fn plan_warm_reaps_invalid_dead_slot() {
        // invalid + sandbox gone → ReapInvalid (delete row, no kill).
        let (actions, shortfall) =
            plan_warm_actions(1, &[(st("invalid"), false), (st("running"), true)]);
        assert_eq!(
            actions,
            vec![WarmWorkerAction::ReapInvalid, WarmWorkerAction::Renew]
        );
        assert_eq!(shortfall, 0);
    }

    #[test]
    fn plan_warm_full_has_zero_shortfall() {
        // W1: two usable + poolSize 2 → no create.
        let (actions, shortfall) =
            plan_warm_actions(2, &[(st("running"), true), (st("running"), true)]);
        assert_eq!(
            actions,
            vec![WarmWorkerAction::Renew, WarmWorkerAction::Renew]
        );
        assert_eq!(shortfall, 0);
    }

    #[test]
    fn plan_warm_invalid_does_not_count_toward_fullness() {
        // W4: invalid occupies a slot, never counts → shortfall creates a fresh slot.
        let (actions, shortfall) =
            plan_warm_actions(1, &[(st("running"), true), (st("invalid"), true)]);
        assert_eq!(
            actions,
            vec![WarmWorkerAction::Renew, WarmWorkerAction::Leave]
        );
        assert_eq!(shortfall, 0);
        // Same but no usable → invalid does not satisfy poolSize.
        let (_, shortfall2) = plan_warm_actions(1, &[(st("invalid"), true)]);
        assert_eq!(shortfall2, 1);
    }

    // ---- reconcile planning (by_buildid, no kill action) ----

    fn row_with(template: &str, lifecycle: &str) -> ProjectFcWorkerRow {
        ProjectFcWorkerRow {
            proj_id: 1,
            scope_key: String::new(),
            slot_index: 0,
            sandbox_id: "sbx".into(),
            worker_id: "w".into(),
            template_id: template.to_string(),
            handle_json: json!({}),
            updated_at_ms: 0,
            in_use_count: 0,
            in_use_until_ms: 0,
            lifecycle_state: lifecycle.to_string(),
            last_idle_at_ms: 0,
            mcp_bind_json: json!({}),
            invalid_reason: String::new(),
        }
    }

    #[test]
    fn plan_reconcile_marks_only_stale_build() {
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        let stale = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let rows = vec![row_with(&stale, "running"), row_with(&desired, "running")];
        let idx = plan_reconcile_invalidations(&desired, &rows);
        assert_eq!(idx, vec![0]);
    }

    #[test]
    fn plan_reconcile_marks_stale_when_desired_build_advanced() {
        // Constraint 1: PG desired build advanced with no publish callback / restart —
        // decision layer alone must mark the old applied build. Author: kejiqing
        let applied = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let desired_after_pg_write = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        let rows = vec![row_with(&applied, "running")];
        assert_eq!(
            plan_reconcile_invalidations(&desired_after_pg_write, &rows),
            vec![0]
        );
    }

    #[test]
    fn plan_reconcile_skips_already_invalid() {
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        let stale = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let rows = vec![row_with(&stale, "invalid")];
        assert!(plan_reconcile_invalidations(&desired, &rows).is_empty());
    }

    #[test]
    fn plan_reconcile_marks_legacy_no_pin() {
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        let legacy = worker_contract_key("tpl_a", None, "rev", "strict");
        let rows = vec![row_with(&legacy, "running")];
        assert_eq!(plan_reconcile_invalidations(&desired, &rows), vec![0]);
    }

    // ---- acquire guard (constraint 2: fly over invalid / stale build) ----

    #[test]
    fn acquire_slot_usable_false_when_lifecycle_invalid() {
        let c = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        assert!(!acquire_slot_usable("invalid", &c, &c));
    }

    #[test]
    fn acquire_slot_usable_false_when_build_stale_even_if_running() {
        let stored = worker_contract_key("tpl_a", Some("b1"), "rev", "strict");
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(!acquire_slot_usable("running", &stored, &desired));
    }

    #[test]
    fn acquire_slot_usable_true_when_running_and_build_matches() {
        let c = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(acquire_slot_usable("running", &c, &c));
        assert!(acquire_slot_usable("sleeping", &c, &c));
    }

    #[test]
    fn acquire_slot_usable_false_when_legacy_no_pin_and_desired_pinned() {
        let legacy = worker_contract_key("tpl_a", None, "rev", "strict");
        let desired = worker_contract_key("tpl_a", Some("b2"), "rev", "strict");
        assert!(!acquire_slot_usable("running", &legacy, &desired));
    }

    #[test]
    fn next_free_slot_skips_occupied() {
        let used: std::collections::HashSet<u32> = [0, 1, 3].into_iter().collect();
        assert_eq!(next_free_slot(&used), 2);
    }

    #[test]
    fn next_free_slot_starts_at_zero() {
        let used = std::collections::HashSet::new();
        assert_eq!(next_free_slot(&used), 0);
    }
}
