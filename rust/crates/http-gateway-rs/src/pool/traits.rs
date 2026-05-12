//! Types shared by pool backends. Author: kejiqing

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Optional read-only host paths rebinding into the session guest root (`/claw_host_root`). kejiqing
#[derive(Clone, Debug, Default)]
pub struct PoolSessionHostMounts {
    /// Host `ds_*/home/skills` directory → guest `.../home/skills:ro`.
    pub skills_dir: Option<PathBuf>,
    /// Host `ds_*/CLAUDE.md` file → guest `.../CLAUDE.md:ro`.
    pub claude_md_file: Option<PathBuf>,
}

/// Lease for one worker slot (index into the pool).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotLease {
    pub slot_index: usize,
}

/// Result of `docker exec` (or equivalent) running `claw gateway-solve-once`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Abstract pool: in-process [`super::DockerPoolManager`] or host RPC client. Author: kejiqing
#[async_trait]
pub trait PoolOps: Send + Sync {
    /// `host_mounts`: optional read-only binds for ds-level `home/skills` and root `CLAUDE.md`
    /// (no per-session copy; see [`PoolSessionHostMounts`]).
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
    ) -> Result<TaskOutcome, String>;

    async fn release_slot(&self, slot: SlotLease) -> Result<(), String>;

    async fn force_kill_slot(&self, slot_index: usize) -> Result<(), String>;
}
