//! Pool construction from env or explicit config (tests inject a fake `docker`). Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;

use claw_sandbox_protocol::IsolationMode;

use super::live_report_hub::LiveReportHub;
use super::worker_identity::PoolWorkerIdentity;

/// `CLAW_POOL_WORKER_ISOLATION` — fixed profile for this pool daemon (`strict` / `relaxed`). Author: kejiqing
#[must_use]
pub fn fixed_isolation_from_env() -> Option<IsolationMode> {
    match std::env::var("CLAW_POOL_WORKER_ISOLATION") {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "strict" => Some(IsolationMode::Strict),
            "relaxed" => Some(IsolationMode::Relaxed),
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
    /// Relaxed-profile slot cap when this daemon serves multiple profiles (unified pool). Author: kejiqing
    pub relaxed_pool_size: usize,
    pub relaxed_min_idle: usize,
    pub image: String,
    /// Relaxed-profile image; defaults to `image` when unset.
    pub relaxed_image: String,
    pub network_args: Vec<String>,
    pub extra_run_args: Vec<String>,
    /// If `None`, a random 8-char stem is generated (production).
    pub name_stem: Option<String>,
    pub on_release_exec: Option<String>,
    /// Overrides login name; when `None`, exec uses `worker_identity.exec_user_arg()` (`uid:gid`).
    pub exec_user: Option<String>,
    pub worker_identity: PoolWorkerIdentity,
    pub security_boost: bool,
    /// When set, this daemon only runs one worker profile (dual-pool deploy). Author: kejiqing
    pub fixed_isolation: Option<IsolationMode>,
    /// Symlink guest inject (fake-docker unit tests only; production uses tmpfs).
    pub symlink_inject: bool,
    pub worker_env_host_file: Option<PathBuf>,
    /// Pool-local live report hub (required on `claw-pool-daemon`).
    pub live_report_hub: Option<Arc<LiveReportHub>>,
    /// Set on `claw-pool-daemon` for `claw_pool` registry. Author: kejiqing
    pub pool_id: Option<String>,
}

impl std::fmt::Debug for DockerPoolConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DockerPoolConfig")
            .field("runtime_bin", &self.runtime_bin)
            .field("work_root", &self.work_root)
            .field("pool_size", &self.pool_size)
            .field("min_idle", &self.min_idle)
            .field("image", &self.image)
            .field("relaxed_image", &self.relaxed_image)
            .field("pool_id", &self.pool_id)
            .finish_non_exhaustive()
    }
}

impl DockerPoolConfig {
    pub fn validate(&self) -> Result<(), String> {
        let (strict_max, strict_idle, relaxed_max, relaxed_idle) = profile_pool_limits(self);
        if strict_max + relaxed_max == 0 {
            return Err("at least one worker profile must have pool_size >= 1".to_string());
        }
        if strict_max > 0 && strict_idle > strict_max {
            return Err(format!(
                "min_idle ({strict_idle}) must be <= strict pool_size ({strict_max})"
            ));
        }
        if relaxed_max > 0 && relaxed_idle > relaxed_max {
            return Err(format!(
                "relaxed_min_idle ({relaxed_idle}) must be <= relaxed pool_size ({relaxed_max})"
            ));
        }
        if self.image.trim().is_empty() {
            return Err("image must be non-empty".to_string());
        }
        if self.relaxed_image.trim().is_empty() {
            return Err("relaxed_image must be non-empty".to_string());
        }
        if self.runtime_bin.trim().is_empty() {
            return Err("runtime_bin must be non-empty".to_string());
        }
        Ok(())
    }
}

/// Resolve per-profile slot limits from config + optional fixed profile. Author: kejiqing
#[must_use]
pub fn profile_pool_limits(cfg: &DockerPoolConfig) -> (usize, usize, usize, usize) {
    match cfg.fixed_isolation {
        Some(IsolationMode::Strict) => (cfg.pool_size, cfg.min_idle, 0, 0),
        Some(IsolationMode::Relaxed) => (0, 0, cfg.pool_size, cfg.min_idle),
        None => (
            cfg.pool_size,
            cfg.min_idle,
            cfg.relaxed_pool_size,
            cfg.relaxed_min_idle,
        ),
    }
}
