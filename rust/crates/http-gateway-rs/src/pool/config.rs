//! Pool construction from env or explicit config (tests inject a fake `docker`). Author: kejiqing

use std::path::PathBuf;

/// Snapshot of pool parameters (read once at construction; no hot reload).
#[derive(Debug, Clone)]
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
    /// When a slot returns from `Leased` to `Idle`, run this shell inside the worker via
    /// `sh -lc` (e.g. `pkill -f pattern` for stray `run_in_background` children). Set from env
    /// `CLAW_DOCKER_POOL_ON_RELEASE` / `CLAW_PODMAN_POOL_ON_RELEASE`. Empty / unset = skip.
    pub on_release_exec: Option<String>,
    /// `docker exec --user …` for solve (`gateway-solve-once`). E.g. `claw` or `1000:1000`. Unset =
    /// run as container default (usually root). Match worker image user; host `work_root` should
    /// allow writes for that uid. `POOL_ON_RELEASE` runs **without** this (as root) so cleanup
    /// can `pkill -u claw` etc.
    pub exec_user: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn validate_rejects_zero_pool_size() {
        let c = DockerPoolConfig {
            runtime_bin: "docker".into(),
            work_root: PathBuf::from("/tmp"),
            pool_size: 0,
            min_idle: 0,
            image: "x".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("ab".into()),
            on_release_exec: None,
            exec_user: None,
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn validate_rejects_min_idle_gt_pool() {
        let c = DockerPoolConfig {
            runtime_bin: "docker".into(),
            work_root: PathBuf::from("/tmp"),
            pool_size: 1,
            min_idle: 2,
            image: "x".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("ab".into()),
            on_release_exec: None,
            exec_user: None,
        };
        assert!(c.validate().is_err());
    }
}
