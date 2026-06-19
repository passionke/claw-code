//! FC cloud sandbox pool backend for per-project `mode=sandbox` solve. Author: kejiqing

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use claw_fc_sandbox_client::FcSandboxClient;
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

use crate::session_db::GatewaySessionDb;

use super::merge_stdout_hooks;
use super::result::parse_gateway_solve_exec_stdout;
use super::session_db_sync::{
    finalize_turn_after_readback, materialize_turn_via_sandbox_host_paths, proj_work_dir,
    readback_turn_from_session_home, MaterializeInput,
};
use super::traits::{PoolOps, SlotLease, TaskOutcome};
use super::LiveReportHub;

pub const FC_POOL_ID: &str = "fc-cloud";

struct FcSlot {
    sandbox_id: String,
}

/// Per-turn FC sandbox leases (synthetic slot indices). Author: kejiqing
pub struct FcOrchestratedPool {
    client: Arc<FcSandboxClient>,
    work_root: PathBuf,
    db: RwLock<Option<Arc<GatewaySessionDb>>>,
    slots: Mutex<HashMap<usize, FcSlot>>,
    turn_slots: Mutex<HashMap<String, usize>>,
    next_slot: AtomicUsize,
    live_report_hub: Arc<LiveReportHub>,
}

impl FcOrchestratedPool {
    #[must_use]
    pub fn new(
        client: Arc<FcSandboxClient>,
        work_root: PathBuf,
        live_report_hub: Arc<LiveReportHub>,
    ) -> Self {
        Self {
            client,
            work_root,
            db: RwLock::new(None),
            slots: Mutex::new(HashMap::new()),
            turn_slots: Mutex::new(HashMap::new()),
            next_slot: AtomicUsize::new(1),
            live_report_hub,
        }
    }

    #[must_use]
    pub fn pool_id(&self) -> &'static str {
        FC_POOL_ID
    }

    pub async fn bind_session_db(&self, db: Arc<GatewaySessionDb>) {
        *self.db.write().await = Some(db);
    }

    async fn session_db(&self) -> Result<Arc<GatewaySessionDb>, String> {
        self.db
            .read()
            .await
            .clone()
            .ok_or_else(|| "fc pool: session db not bound".into())
    }

    fn alloc_slot_index(&self) -> usize {
        self.next_slot.fetch_add(1, Ordering::Relaxed)
    }
}

#[async_trait]
impl PoolOps for FcOrchestratedPool {
    async fn acquire_slot(
        &self,
        _wait: Duration,
        session_id: String,
        proj_id: i64,
        turn_id: String,
    ) -> Result<SlotLease, String> {
        let db = self.session_db().await?;
        db.assert_session_can_acquire_for_turn(&session_id, proj_id, &turn_id)
            .await
            .map_err(|reason| format!("session acquire blocked: {reason}"))?;

        let handle = self
            .client
            .create_sandbox(&session_id, proj_id, true)
            .await?;
        let slot_index = self.alloc_slot_index();
        let worker_name = format!("fc:{}", handle.sandbox_id);
        let _ = db
            .assign_turn_pool_worker(&turn_id, FC_POOL_ID, &worker_name, Some("0:0"))
            .await;

        let proj_work_dir = proj_work_dir(&self.work_root, proj_id);
        materialize_turn_via_sandbox_host_paths(
            db.as_ref(),
            &self.work_root,
            &proj_work_dir,
            &MaterializeInput {
                session_id: session_id.clone(),
                proj_id,
                turn_id: turn_id.clone(),
            },
        )
        .await?;

        self.slots.lock().await.insert(
            slot_index,
            FcSlot {
                sandbox_id: handle.sandbox_id.clone(),
            },
        );
        self.turn_slots.lock().await.insert(turn_id, slot_index);
        Ok(SlotLease { slot_index })
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
        let sandbox_id = self
            .slots
            .lock()
            .await
            .get(&slot.slot_index)
            .map(|s| s.sandbox_id.clone())
            .ok_or_else(|| format!("fc slot {} not found", slot.slot_index))?;

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
            .exec_gateway_solve_once(
                &sandbox_id,
                task_rel_under_root,
                claw_bin,
                worker_llm_env.unwrap_or_default(),
            )
            .await?;

        if let Some(ref hook) = stdout_hook {
            for line in outcome.stdout.lines() {
                hook(line.to_string());
            }
        }

        let task_outcome = TaskOutcome {
            exit_code: outcome.exit_code,
            stdout: outcome.stdout.clone(),
            stderr: outcome.stderr.clone(),
        };

        if task_outcome.exit_code == 0 {
            if let Ok(Some((session_id, proj_id))) = db.turn_session_scope(turn_id).await {
                let user_prompt = db
                    .get_turn_user_prompt(turn_id)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if let Err(e) = readback_turn_from_session_home(
                    db.as_ref(),
                    db.pg_pool(),
                    &self.work_root,
                    &session_id,
                    proj_id,
                    turn_id,
                    &user_prompt,
                )
                .await
                {
                    warn!(
                        target: "claw_gateway_fc_pool",
                        turn_id = %turn_id,
                        error = %e,
                        "fc readback from session home failed"
                    );
                } else {
                    let parsed = parse_gateway_solve_exec_stdout(
                        &task_outcome.stdout,
                        task_outcome.exit_code,
                    );
                    let report = parsed.output_json.as_ref().and_then(|j| {
                        crate::biz_advice_report::report_body_from_solve_output(
                            &parsed.output_text,
                            Some(j),
                        )
                        .ok()
                    });
                    let _ = finalize_turn_after_readback(
                        db.as_ref(),
                        turn_id,
                        parsed.claw_exit_code,
                        report.as_deref(),
                        parsed.output_json.as_ref(),
                    )
                    .await;
                }
            }
        }

        Ok(task_outcome)
    }

    async fn release_slot(&self, slot: SlotLease) -> Result<(), String> {
        let sandbox_id = self
            .slots
            .lock()
            .await
            .remove(&slot.slot_index)
            .map(|s| s.sandbox_id);
        self.turn_slots
            .lock()
            .await
            .retain(|_, idx| *idx != slot.slot_index);
        if let Some(id) = sandbox_id {
            self.client.kill_sandbox(&id).await?;
        }
        Ok(())
    }

    async fn force_kill_slot(&self, slot_index: usize) -> Result<(), String> {
        self.release_slot(SlotLease { slot_index }).await
    }

    async fn has_report_for_turn(&self, turn_id: &str) -> bool {
        self.live_report_hub.has_report_for_turn(turn_id)
    }

    async fn first_report_at_ms_for_turn(&self, turn_id: &str) -> Option<i64> {
        self.live_report_hub.first_report_at_ms_for_turn(turn_id)
    }
}
