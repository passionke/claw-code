//! In-process pool backend (tests). Author: kejiqing

use std::sync::Arc;
use std::time::Duration;

use claw_sandbox_protocol::{IsolationMode, SlotLease, TaskOutcome};

use super::docker_pool::DockerPoolManager;

/// Direct access to an in-process [`DockerPoolManager`] for unit tests.
pub struct LocalPoolOps(pub Arc<DockerPoolManager>);

impl LocalPoolOps {
    pub async fn acquire_slot(&self, wait: Duration) -> Result<SlotLease, String> {
        self.0.acquire_slot(wait, IsolationMode::Strict).await
    }

    pub async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        turn_id: &str,
        worker_llm_env: Option<std::collections::BTreeMap<String, String>>,
    ) -> Result<TaskOutcome, String> {
        self.0
            .exec_solve(
                slot,
                task_rel_under_root,
                claw_bin,
                None,
                turn_id,
                worker_llm_env,
                None,
            )
            .await
    }

    pub async fn release_slot(&self, slot: SlotLease) -> Result<(), String> {
        self.0.release_slot(slot).await
    }
}
