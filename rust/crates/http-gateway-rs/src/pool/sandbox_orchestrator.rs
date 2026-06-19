//! Gateway-side sandbox pool orchestration (PG materialize/readback via RPC). Author: kejiqing

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use claw_sandbox_client::SandboxRpcClient;
use claw_sandbox_protocol::IsolationMode as SandboxIsolationMode;
use tokio::sync::{Mutex, RwLock};

use crate::session_db::GatewaySessionDb;

use super::merge_stdout_hooks;
use super::session_db_sync::{
    finalize_turn_after_readback, materialize_turn_via_sandbox, readback_turn_via_sandbox,
    sync_progress_via_sandbox, MaterializeInput,
};
use super::traits::{PoolOps, SlotLease, TaskOutcome};
use super::worker_isolation::{default_worker_isolation_json, mode_from_json, WorkerIsolationMode};
use super::LiveReportHub;

/// Maps gateway worker isolation to sandbox acquire isolation. Author: kejiqing
#[must_use]
pub fn worker_isolation_to_sandbox(mode: WorkerIsolationMode) -> SandboxIsolationMode {
    match mode {
        WorkerIsolationMode::Strict => SandboxIsolationMode::Strict,
        WorkerIsolationMode::Relaxed => SandboxIsolationMode::Relaxed,
    }
}

/// Pool backend: HTTP sandbox RPC + gateway PG orchestration. Author: kejiqing
pub struct SandboxOrchestratedPool {
    client: Arc<SandboxRpcClient>,
    db: Arc<RwLock<Option<Arc<GatewaySessionDb>>>>,
    work_root: PathBuf,
    pool_id: String,
    live_report_hub: Arc<LiveReportHub>,
    turn_slots: Arc<Mutex<HashMap<String, usize>>>,
    slot_workers: Arc<Mutex<HashMap<usize, String>>>,
}

impl SandboxOrchestratedPool {
    #[must_use]
    pub fn new(
        client: Arc<SandboxRpcClient>,
        work_root: PathBuf,
        pool_id: String,
        live_report_hub: Arc<LiveReportHub>,
    ) -> Arc<Self> {
        Arc::new(Self {
            client,
            db: Arc::new(RwLock::new(None)),
            work_root,
            pool_id,
            live_report_hub,
            turn_slots: Arc::new(Mutex::new(HashMap::new())),
            slot_workers: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        *self.db.write().await = Some(db);
    }

    async fn session_db(&self) -> Result<Arc<GatewaySessionDb>, String> {
        self.db
            .read()
            .await
            .clone()
            .ok_or_else(|| "sandbox pool: session_db not bound".into())
    }

    #[must_use]
    pub fn rpc_client(&self) -> Arc<SandboxRpcClient> {
        Arc::clone(&self.client)
    }

    async fn resolve_isolation(&self, proj_id: i64) -> Result<SandboxIsolationMode, String> {
        let db = self.session_db().await?;
        let json = db
            .get_worker_isolation_json(proj_id)
            .await
            .unwrap_or_else(|_| default_worker_isolation_json());
        if super::worker_isolation::is_fc_sandbox_mode(&json) {
            return Err(
                "internal: podman pool acquire for proj with worker_isolation_json.mode=sandbox"
                    .into(),
            );
        }
        Ok(worker_isolation_to_sandbox(mode_from_json(&json)))
    }

    async fn slot_index_for_turn(&self, turn_id: &str) -> Result<usize, String> {
        if let Some(idx) = self.turn_slots.lock().await.get(turn_id).copied() {
            return Ok(idx);
        }
        let db = self.session_db().await?;
        let worker = db
            .get_turn_worker_name(turn_id)
            .await
            .map_err(|e| format!("get worker_name: {e}"))?
            .filter(|w| !w.trim().is_empty())
            .ok_or_else(|| format!("no slot mapping for turn {turn_id}"))?;
        slot_index_from_worker_name(&worker)
            .ok_or_else(|| format!("cannot parse slot_index from worker {worker}"))
    }
}

/// Parse `claw-worker-{stem}-{profile}-{n}` global slot suffix. Author: kejiqing
fn slot_index_from_worker_name(worker_name: &str) -> Option<usize> {
    worker_name
        .rsplit_once('-')
        .and_then(|(_, idx)| idx.parse().ok())
}

#[async_trait]
impl PoolOps for SandboxOrchestratedPool {
    async fn acquire_slot(
        &self,
        wait: Duration,
        session_id: String,
        proj_id: i64,
        turn_id: String,
    ) -> Result<SlotLease, String> {
        let db = self.session_db().await?;
        db.assert_session_can_acquire_for_turn(&session_id, proj_id, &turn_id)
            .await
            .map_err(|reason| format!("session acquire blocked: {reason}"))?;
        let isolation = self.resolve_isolation(proj_id).await?;
        if isolation == SandboxIsolationMode::Relaxed
            && !super::config::relaxed_worker_allowed_from_env()
        {
            return Err(
                "proj worker isolation is relaxed but CLAW_ALLOW_RELAXED_WORKER=false on gateway"
                    .into(),
            );
        }
        let lease = self
            .client
            .acquire(
                wait,
                isolation,
                None,
                Some(claw_sandbox_protocol::SlotLeaseOwner::Solve {
                    turn_id: turn_id.clone(),
                    proj_id,
                }),
            )
            .await?;
        if let Some(ref worker_name) = lease.worker_name {
            let exec_user = lease.exec_identity.as_ref().map(|id| id.exec_user.as_str());
            let _ = db
                .assign_turn_pool_worker(&turn_id, &self.pool_id, worker_name, exec_user)
                .await;
        }
        let proj_work_dir = super::session_db_sync::proj_work_dir(&self.work_root, proj_id);
        materialize_turn_via_sandbox(
            &self.client,
            lease.slot_index,
            db.as_ref(),
            &proj_work_dir,
            &MaterializeInput {
                session_id: session_id.clone(),
                proj_id,
                turn_id: turn_id.clone(),
            },
        )
        .await?;
        if let Some(ref worker_name) = lease.worker_name {
            self.slot_workers
                .lock()
                .await
                .insert(lease.slot_index, worker_name.clone());
        }
        self.turn_slots
            .lock()
            .await
            .insert(turn_id, lease.slot_index);
        Ok(SlotLease {
            slot_index: lease.slot_index,
        })
    }

    async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        _request_id: Option<&str>,
        turn_id: &str,
        worker_llm_env: Option<BTreeMap<String, String>>,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TaskOutcome, String> {
        let db = self.session_db().await?;
        self.turn_slots
            .lock()
            .await
            .insert(turn_id.to_string(), slot.slot_index);
        let stdout_hook = merge_stdout_hooks(
            turn_id,
            Some(Arc::clone(&self.live_report_hub)),
            on_stdout_line,
        );
        let outcome = self
            .client
            .exec_solve(
                slot.slot_index,
                task_rel_under_root,
                claw_bin,
                turn_id,
                worker_llm_env,
                stdout_hook,
            )
            .await?;
        if outcome.exit_code == 0 {
            if let Ok(Some((session_id, proj_id))) = db.turn_session_scope(turn_id).await {
                let user_prompt = db
                    .get_turn_user_prompt(turn_id)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                readback_turn_via_sandbox(
                    &self.client,
                    slot.slot_index,
                    db.as_ref(),
                    db.pg_pool(),
                    &session_id,
                    proj_id,
                    turn_id,
                    &user_prompt,
                )
                .await
                .map_err(|e| format!("readback_turn_via_sandbox failed: {e}"))?;
                let parsed = super::result::parse_gateway_solve_exec_stdout(
                    &outcome.stdout,
                    outcome.exit_code,
                );
                let report = parsed.output_json.as_ref().and_then(|j| {
                    crate::biz_advice_report::report_body_from_solve_output(
                        &parsed.output_text,
                        Some(j),
                    )
                    .ok()
                });
                finalize_turn_after_readback(
                    db.as_ref(),
                    turn_id,
                    parsed.claw_exit_code,
                    report.as_deref(),
                    parsed.output_json.as_ref(),
                )
                .await
                .map_err(|e| format!("finalize_turn_after_readback failed: {e}"))?;
            }
        }
        Ok(TaskOutcome {
            exit_code: outcome.exit_code,
            stdout: outcome.stdout,
            stderr: outcome.stderr,
        })
    }

    async fn release_slot(&self, slot: SlotLease) -> Result<(), String> {
        self.client.release(slot.slot_index).await?;
        self.slot_workers.lock().await.remove(&slot.slot_index);
        let mut turns = self.turn_slots.lock().await;
        turns.retain(|_, idx| *idx != slot.slot_index);
        Ok(())
    }

    async fn force_kill_slot(&self, slot_index: usize) -> Result<(), String> {
        self.client.force_kill(slot_index).await?;
        self.slot_workers.lock().await.remove(&slot_index);
        let mut turns = self.turn_slots.lock().await;
        turns.retain(|_, idx| *idx != slot_index);
        Ok(())
    }

    async fn sync_turn_progress_to_db(&self, turn_id: &str) -> Result<(), String> {
        let db = self.session_db().await?;
        let slot_index = self.slot_index_for_turn(turn_id).await?;
        sync_progress_via_sandbox(&self.client, slot_index, db.as_ref(), turn_id).await
    }

    async fn has_report_for_turn(&self, turn_id: &str) -> bool {
        self.live_report_hub.has_report_for_turn(turn_id)
    }

    async fn first_report_at_ms_for_turn(&self, turn_id: &str) -> Option<i64> {
        self.live_report_hub.first_report_at_ms_for_turn(turn_id)
    }
}
