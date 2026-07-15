//! Types shared by pool backends. Author: kejiqing

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

/// Lease for one worker slot (index into the pool).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SlotLease {
    pub slot_index: usize,
}

/// Result of `docker exec` (or equivalent) running `claw gateway-solve-once`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Abstract solve pool; the supported implementation is e2b cloud sandbox. Author: kejiqing
#[async_trait]
pub trait PoolOps: Send + Sync {
    async fn acquire_slot(
        &self,
        wait: Duration,
        session_id: String,
        proj_id: i64,
        turn_id: String,
    ) -> Result<SlotLease, String>;

    async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        request_id: Option<&str>,
        turn_id: &str,
        worker_llm_env: Option<BTreeMap<String, String>>,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TaskOutcome, String>;

    async fn release_slot(&self, slot: SlotLease) -> Result<(), String>;

    async fn force_kill_slot(&self, slot_index: usize) -> Result<(), String>;

    /// Whether this turn has observed at least one stdout `report.delta`.
    async fn has_report_for_turn(&self, _turn_id: &str) -> bool {
        false
    }

    /// First observed stdout `report.delta` timestamp for the turn.
    async fn first_report_at_ms_for_turn(&self, _turn_id: &str) -> Option<i64> {
        None
    }

    /// Running turn: pull session `.claw` progress into PG (e2b: nas-api). Author: kejiqing
    async fn sync_turn_progress_to_db(&self, _turn_id: &str) -> Result<(), String> {
        Ok(())
    }
}
