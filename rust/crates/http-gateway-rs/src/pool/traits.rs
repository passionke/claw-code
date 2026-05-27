//! Types shared by pool backends. Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

/// Optional read-only host paths rebinding into the session guest root (`/claw_host_root`). kejiqing
#[derive(Clone, Debug, Default)]
pub struct PoolSessionHostMounts {
    /// Host `ds_*/home/skills` directory → guest `.../home/skills:ro`.
    pub skills_dir: Option<PathBuf>,
    /// Host `ds_*/CLAUDE.md` file → guest `.../CLAUDE.md:ro`.
    pub claude_md_file: Option<PathBuf>,
    /// Host `ds_*/home/schema.md` (or legacy catalog) → guest `.../home/schema.md:ro`. kejiqing
    pub data_catalog_file: Option<PathBuf>,
    /// Host `ds_*/home/.claw/solve-preflight.json` → guest `.../home/.claw/solve-preflight.json:ro`. kejiqing
    pub solve_preflight_file: Option<PathBuf>,
    /// Host `ds_*/home/.claw/solve-orchestration.json` → guest `.../home/.claw/solve-orchestration.json:ro`. kejiqing
    pub solve_orchestration_file: Option<PathBuf>,
}

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

/// Abstract pool: host [`super::rpc::PoolRpcClient`] talking to `claw-pool-daemon`. Author: kejiqing
#[async_trait]
pub trait PoolOps: Send + Sync {
    async fn acquire_slot(
        &self,
        wait: Duration,
        session_host_mount: PathBuf,
        host_mounts: PoolSessionHostMounts,
    ) -> Result<SlotLease, String>;

    async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        request_id: Option<&str>,
        turn_id: &str,
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
}
