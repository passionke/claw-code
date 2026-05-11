//! In-process pool as [`PoolOps`] (wraps [`super::DockerPoolManager`]). Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::docker_pool::DockerPoolManager;
use super::traits::{PoolOps, SlotLease, TaskOutcome};

/// Adapter so [`Arc<dyn PoolOps>`] can wrap the local [`DockerPoolManager`]. Author: kejiqing
pub struct LocalPoolOps(pub Arc<DockerPoolManager>);

#[async_trait]
impl PoolOps for LocalPoolOps {
    async fn acquire_slot(
        &self,
        wait: Duration,
        session_host_mount: PathBuf,
    ) -> Result<SlotLease, String> {
        self.0.acquire_slot(wait, session_host_mount).await
    }

    async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        request_id: Option<&str>,
    ) -> Result<TaskOutcome, String> {
        self.0
            .exec_solve(slot, task_rel_under_root, claw_bin, request_id)
            .await
    }

    async fn release_slot(&self, slot: SlotLease) -> Result<(), String> {
        self.0.release_slot(slot).await
    }

    async fn force_kill_slot(&self, slot_index: usize) -> Result<(), String> {
        self.0.force_kill_slot(slot_index).await
    }
}
