//! In-process pool backend (tests). Author: kejiqing

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::docker_pool::DockerPoolManager;
use super::traits::{PoolOps, SlotLease, TaskOutcome};

/// [`PoolOps`] backed by an in-process [`DockerPoolManager`]. Author: kejiqing
pub struct LocalPoolOps(pub Arc<DockerPoolManager>);

#[async_trait]
impl PoolOps for LocalPoolOps {
    async fn acquire_slot(
        &self,
        wait: Duration,
        session_id: String,
        ds_id: i64,
        turn_id: String,
    ) -> Result<SlotLease, String> {
        self.0.acquire_slot(wait, session_id, ds_id, turn_id).await
    }

    async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        request_id: Option<&str>,
        turn_id: &str,
        worker_llm_env: Option<std::collections::BTreeMap<String, String>>,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TaskOutcome, String> {
        self.0
            .exec_solve(
                slot,
                task_rel_under_root,
                claw_bin,
                request_id,
                turn_id,
                worker_llm_env,
                on_stdout_line,
            )
            .await
    }

    async fn release_slot(&self, slot: SlotLease) -> Result<(), String> {
        self.0.release_slot(slot).await
    }

    async fn force_kill_slot(&self, slot_index: usize) -> Result<(), String> {
        self.0.force_kill_slot(slot_index).await
    }

    async fn has_report_for_turn(&self, turn_id: &str) -> bool {
        self.0.has_report_for_turn(turn_id)
    }

    async fn first_report_at_ms_for_turn(&self, turn_id: &str) -> Option<i64> {
        self.0.first_report_at_ms_for_turn(turn_id)
    }
}
