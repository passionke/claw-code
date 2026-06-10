//! Pool construction from env or explicit config (tests inject a fake `docker`). Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;

use crate::session_db::GatewaySessionDb;

use super::live_report_hub::LiveReportHub;
use super::worker_identity::PoolWorkerIdentity;
use super::worker_isolation::WorkerIsolationMode;

/// `CLAW_POOL_WORKER_ISOLATION` — fixed profile for this pool daemon (`strict` / `relaxed`). Author: kejiqing
#[must_use]
pub fn fixed_isolation_from_env() -> Option<WorkerIsolationMode> {
    match std::env::var("CLAW_POOL_WORKER_ISOLATION") {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "strict" => Some(WorkerIsolationMode::Strict),
            "relaxed" => Some(WorkerIsolationMode::Relaxed),
            _ => None,
        },
        Err(_) => None,
    }
}

/// `CLAW_ALLOW_RELAXED_WORKER` — when false, all ds use strict profile. Author: kejiqing
#[must_use]
pub fn relaxed_worker_allowed_from_env() -> bool {
    match std::env::var("CLAW_ALLOW_RELAXED_WORKER") {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            !(t == "0" || t == "false" || t == "no" || t == "off")
        }
        Err(_) => true,
    }
}

/// `CLAW_SECURITY_BOOST` — default on. Author: kejiqing
#[must_use]
pub fn security_boost_from_env() -> bool {
    match std::env::var("CLAW_SECURITY_BOOST") {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            !(t == "0" || t == "false" || t == "no" || t == "off")
        }
        Err(_) => true,
    }
}

/// Snapshot of pool parameters (read once at construction; no hot reload).
#[derive(Clone)]
pub struct DockerPoolConfig {
    /// `docker` / `podman` or path to a test stub.
    pub runtime_bin: String,
    pub work_root: PathBuf,
    pub pool_size: usize,
    pub min_idle: usize,
    pub image: String,
    pub network_args: Vec<String>,
    pub extra_run_args: Vec<String>,
    /// If `None`, a random 8-char stem is generated (production).
    pub name_stem: Option<String>,
    pub on_release_exec: Option<String>,
    /// Overrides login name; when `None`, exec uses `worker_identity.exec_user_arg()` (`uid:gid`).
    pub exec_user: Option<String>,
    pub worker_identity: PoolWorkerIdentity,
    pub security_boost: bool,
    /// When set, sandbox only runs one worker profile (tests). Author: kejiqing
    pub fixed_isolation: Option<WorkerIsolationMode>,
    /// Symlink guest inject (fake-docker unit tests only; production uses bind → guest).
    pub symlink_inject: bool,
    pub worker_env_host_file: Option<PathBuf>,
    /// Pool-local live report hub (required on `claw-pool-daemon`).
    pub live_report_hub: Option<Arc<LiveReportHub>>,
    /// Set on `claw-pool-daemon` for `claw_pool` registry and turn assignment. Author: kejiqing
    pub pool_id: Option<String>,
    pub session_db: Option<Arc<GatewaySessionDb>>,
}

impl std::fmt::Debug for DockerPoolConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerPoolConfig")
            .field("runtime_bin", &self.runtime_bin)
            .field("work_root", &self.work_root)
            .field("pool_size", &self.pool_size)
            .field("min_idle", &self.min_idle)
            .field("image", &self.image)
            .field("pool_id", &self.pool_id)
            .field(
                "session_db",
                &self.session_db.as_ref().map(|_| "Some(GatewaySessionDb)"),
            )
            .finish_non_exhaustive()
    }
}

impl DockerPoolConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.pool_size == 0 {
            return Err("pool_size must be >= 1".to_string());
        }
        if self.min_idle > self.pool_size {
            return Err(format!(
                "min_idle ({}) must be <= pool_size ({})",
                self.min_idle, self.pool_size
            ));
        }
        if self.image.trim().is_empty() {
            return Err("image must be non-empty".to_string());
        }
        if self.runtime_bin.trim().is_empty() {
            return Err("runtime_bin must be non-empty".to_string());
        }
        Ok(())
    }
}
