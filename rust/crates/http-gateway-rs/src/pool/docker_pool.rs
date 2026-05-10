//! Docker/Podman worker pool: env read once at construction; internal `ensure_warm`. Author: kejiqing

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tracing::{info, warn};
use uuid::Uuid;

use super::config::DockerPoolConfig;
use super::docker_cli::runtime_exec;
use super::{SlotLease, TaskOutcome};

pub const GUEST_WORK_ROOT: &str = "/claw_host_root";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Idle,
    Leased,
    Dead,
}

#[derive(Debug, Clone)]
struct Slot {
    container_name: String,
    state: SlotState,
}

/// Pool of long-lived worker containers; host `work_root` mounted at [`GUEST_WORK_ROOT`].
pub struct DockerPoolManager {
    name_stem: String,
    bin: String,
    slots: Mutex<Vec<Slot>>,
    work_root_host: PathBuf,
    pool_size: usize,
    min_idle: usize,
    image: String,
    network_args: Vec<String>,
    extra_run_args: Vec<String>,
}

impl DockerPoolManager {
    /// Build pool from explicit config (tests inject a fake `docker` binary path).
    pub fn from_config(cfg: DockerPoolConfig) -> Result<Arc<Self>, String> {
        cfg.validate()?;
        let work_root_host = std::fs::canonicalize(&cfg.work_root)
            .map_err(|e| format!("canonicalize work_root {}: {e}", cfg.work_root.display()))?;
        let name_stem = cfg.name_stem.unwrap_or_else(|| {
            let u = Uuid::new_v4().simple().to_string();
            u[..8].to_string()
        });
        Ok(Arc::new(Self {
            name_stem,
            bin: cfg.runtime_bin,
            slots: Mutex::new(Vec::new()),
            work_root_host,
            pool_size: cfg.pool_size,
            min_idle: cfg.min_idle,
            image: cfg.image,
            network_args: cfg.network_args,
            extra_run_args: cfg.extra_run_args,
        }))
    }

    /// Read `CLAW_DOCKER_POOL_*` or `CLAW_PODMAN_POOL_*` once at construction.
    pub fn try_from_env(podman: bool, work_root: &Path) -> Result<Arc<Self>, String> {
        let (default_bin, pfx) = if podman {
            ("podman", "CLAW_PODMAN_")
        } else {
            ("docker", "CLAW_DOCKER_")
        };
        let pool_size = std::env::var(format!("{pfx}POOL_SIZE"))
            .unwrap_or_else(|_| "2".to_string())
            .parse::<usize>()
            .map_err(|_| format!("{pfx}POOL_SIZE must be a positive integer"))?;
        let min_idle = std::env::var(format!("{pfx}POOL_MIN_IDLE"))
            .unwrap_or_else(|_| "1".to_string())
            .parse::<usize>()
            .map_err(|_| format!("{pfx}POOL_MIN_IDLE must be an integer"))?;
        let image = std::env::var(format!("{pfx}IMAGE"))
            .map_err(|_| format!("{pfx}IMAGE is required for container pool"))?;
        let network_args = match std::env::var(format!("{pfx}NETWORK")) {
            Ok(n) if !n.trim().is_empty() => vec!["--network".to_string(), n.trim().to_string()],
            _ => Vec::new(),
        };
        let extra_run_args = std::env::var(format!("{pfx}EXTRA_ARGS"))
            .map(|s| s.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        Self::from_config(DockerPoolConfig {
            runtime_bin: default_bin.to_string(),
            work_root: work_root.to_path_buf(),
            pool_size,
            min_idle,
            image,
            network_args,
            extra_run_args,
            name_stem: None,
        })
    }

    /// Test hook: run the same warm pass as `schedule_warm` (revive `Dead`, fill `min_idle`).
    #[cfg(test)]
    pub async fn test_ensure_warm_now(self: &Arc<Self>) -> Result<(), String> {
        self.ensure_warm().await
    }

    fn container_name(&self, idx: usize) -> String {
        format!("claw-gw-{}-{idx}", self.name_stem)
    }

    pub fn schedule_warm(self: &Arc<Self>) {
        let s = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = s.ensure_warm().await {
                warn!(error = %e, "pool ensure_warm failed");
            }
        });
    }

    async fn ensure_warm(self: &Arc<Self>) -> Result<(), String> {
        let mut slots = self.slots.lock().await;
        for i in 0..slots.len() {
            if slots[i].state != SlotState::Dead {
                continue;
            }
            let old = slots[i].container_name.clone();
            drop(slots);
            let _ = self.rm_container(&old).await;
            let name = self.container_name(i);
            self.run_worker_container(&name).await?;
            slots = self.slots.lock().await;
            slots[i] = Slot {
                container_name: name,
                state: SlotState::Idle,
            };
        }
        let mut idle = slots.iter().filter(|s| s.state == SlotState::Idle).count();
        let mut total = slots.len();
        while idle < self.min_idle && total < self.pool_size {
            let idx = total;
            let name = self.container_name(idx);
            drop(slots);
            self.run_worker_container(&name).await?;
            slots = self.slots.lock().await;
            slots.push(Slot {
                container_name: name,
                state: SlotState::Idle,
            });
            idle += 1;
            total += 1;
        }
        while slots.len() > self.pool_size {
            let last = slots.len() - 1;
            if slots[last].state == SlotState::Idle {
                let name = slots[last].container_name.clone();
                slots.pop();
                drop(slots);
                let _ = self.rm_container(&name).await;
                slots = self.slots.lock().await;
            } else {
                break;
            }
        }
        Ok(())
    }

    async fn run_worker_container(&self, name: &str) -> Result<(), String> {
        let mut args: Vec<String> = vec![
            "run".into(),
            "-d".into(),
            "--name".into(),
            name.into(),
            "--restart".into(),
            "no".into(),
        ];
        args.extend(self.network_args.iter().cloned());
        args.extend(self.extra_run_args.iter().cloned());
        args.push("-v".into());
        args.push(format!(
            "{}:{}:rw",
            self.work_root_host.display(),
            GUEST_WORK_ROOT
        ));
        args.push(self.image.clone());
        args.push("sleep".into());
        args.push("infinity".into());
        let exec_argv: Vec<&str> = args.iter().map(String::as_str).collect();
        info!(container = %name, bin = %self.bin, "pool run worker");
        let out = runtime_exec(&self.bin, &exec_argv)
            .await
            .map_err(|e| format!("spawn {}: {e}", self.bin))?;
        if !out.status.success() {
            return Err(format!(
                "{} run failed: {}",
                self.bin,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Ok(())
    }

    async fn rm_container(&self, name: &str) -> Result<(), String> {
        let _ = runtime_exec(&self.bin, &["rm", "-f", name]).await;
        Ok(())
    }

    pub async fn acquire_slot(self: &Arc<Self>, wait: Duration) -> Result<SlotLease, String> {
        timeout(wait, async {
            loop {
                let mut slots = self.slots.lock().await;
                if let Some((i, _)) = slots
                    .iter()
                    .enumerate()
                    .find(|(_, s)| s.state == SlotState::Idle)
                {
                    slots[i].state = SlotState::Leased;
                    return Ok(SlotLease { slot_index: i });
                }
                let total = slots.len();
                if total < self.pool_size {
                    let idx = total;
                    let name = self.container_name(idx);
                    drop(slots);
                    match self.run_worker_container(&name).await {
                        Ok(()) => {}
                        Err(e) => {
                            warn!(error = %e, "pool on-demand worker create failed");
                            sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                    }
                    slots = self.slots.lock().await;
                    slots.push(Slot {
                        container_name: name,
                        state: SlotState::Leased,
                    });
                    let i = slots.len() - 1;
                    return Ok(SlotLease { slot_index: i });
                }
                drop(slots);
                sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| "acquire_slot: timeout waiting for idle worker".to_string())?
    }

    pub async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        ds_id: i64,
        claw_bin: &str,
    ) -> Result<TaskOutcome, String> {
        let name = {
            let slots = self.slots.lock().await;
            slots
                .get(slot.slot_index)
                .filter(|s| s.state == SlotState::Leased)
                .map(|s| s.container_name.clone())
                .ok_or_else(|| "invalid or released slot".to_string())?
        };
        let workdir = format!("{GUEST_WORK_ROOT}/ds_{ds_id}");
        let task_path = format!("{GUEST_WORK_ROOT}/{task_rel_under_root}");
        let argv = [
            "exec",
            "-i",
            "-e",
            "CLAW_GATEWAY_WORK_ROOT=/claw_host_root",
            "--workdir",
            workdir.as_str(),
            name.as_str(),
            claw_bin,
            "gateway-solve-once",
            "--task-file",
            task_path.as_str(),
        ];
        let out = runtime_exec(&self.bin, &argv)
            .await
            .map_err(|e| format!("{} exec: {e}", self.bin))?;
        Ok(TaskOutcome {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    pub async fn release_slot(self: &Arc<Self>, slot: SlotLease) -> Result<(), String> {
        let mut slots = self.slots.lock().await;
        let s = slots
            .get_mut(slot.slot_index)
            .ok_or_else(|| "release: bad slot index".to_string())?;
        if s.state == SlotState::Leased {
            s.state = SlotState::Idle;
        }
        drop(slots);
        Self::schedule_warm(self);
        Ok(())
    }

    pub async fn force_kill_slot(self: &Arc<Self>, slot_index: usize) -> Result<(), String> {
        let name = {
            let mut slots = self.slots.lock().await;
            let s = slots
                .get_mut(slot_index)
                .ok_or_else(|| "force_kill: bad slot".to_string())?;
            let n = s.container_name.clone();
            s.state = SlotState::Dead;
            n
        };
        let _ = runtime_exec(&self.bin, &["kill", &name]).await;
        Self::schedule_warm(self);
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod docker_pool_integration_tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use uuid::Uuid;

    use super::DockerPoolManager;
    use crate::pool::config::DockerPoolConfig;
    use std::sync::Arc;
    use std::time::Duration;

    fn fake_docker_script(state_dir: &Path) -> String {
        let d = state_dir.to_string_lossy().replace('\'', "'\"'\"'");
        format!(
            r#"#!/bin/sh
set -eu
d='{d}'
mkdir -p "$d"
log() {{ printf '%s\n' "$*" >>"$d/log.txt"; }}
case "${{1:-}}" in
run)
  log "run:$*"
  exit 0
  ;;
exec)
  log "exec:$*"
  printf '%s\n' '{{"clawExitCode":0,"outputText":"ok","outputJson":null}}'
  exit 0
  ;;
kill)
  log "kill:$*"
  exit 0
  ;;
rm)
  log "rm:$*"
  exit 0
  ;;
*)
  log "unknown:$*"
  exit 1
  ;;
esac
"#
        )
    }

    fn write_executable(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    fn read_log(state: &Path) -> String {
        fs::read_to_string(state.join("log.txt")).unwrap_or_default()
    }

    fn test_layout() -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("http-gw-pool-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let work = base.join("work");
        fs::create_dir_all(&work).unwrap();
        let state = base.join("docker_state");
        fs::create_dir_all(&state).unwrap();
        let bin_path = base.join("fake-docker");
        write_executable(&bin_path, &fake_docker_script(&state));
        (base, work, bin_path)
    }

    #[tokio::test]
    async fn acquire_exec_release_does_not_rm_worker() {
        let (base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(DockerPoolConfig {
            runtime_bin: bin_path.to_string_lossy().into_owned(),
            work_root: work.clone(),
            pool_size: 2,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("tstem".into()),
        })
        .unwrap();
        let lease = pool.acquire_slot(Duration::from_secs(5)).await.unwrap();
        let out = pool
            .exec_solve(&lease, ".claw-gateway-pool-tasks/x.json", 1, "claw")
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0);
        DockerPoolManager::release_slot(&pool, lease).await.unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        let log = read_log(&state_dir);
        assert!(
            log.contains("run:"),
            "expected container create (run), log:\n{log}"
        );
        assert!(log.contains("exec:"), "expected exec solve, log:\n{log}");
        assert!(
            !log.contains("rm:"),
            "release must not destroy the worker (no rm), log:\n{log}"
        );
    }

    #[tokio::test]
    async fn force_kill_then_ensure_warm_runs_rm_and_new_run() {
        let (base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(DockerPoolConfig {
            runtime_bin: bin_path.to_string_lossy().into_owned(),
            work_root: work,
            pool_size: 2,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("killme".into()),
        })
        .unwrap();
        let lease = pool.acquire_slot(Duration::from_secs(5)).await.unwrap();
        let idx = lease.slot_index;
        pool.force_kill_slot(idx).await.unwrap();
        pool.test_ensure_warm_now().await.unwrap();
        let log = read_log(&state_dir);
        assert!(
            log.contains("kill:"),
            "expected kill after force_kill_slot, log:\n{log}"
        );
        assert!(
            log.contains("rm:"),
            "expected rm when reviving Dead slot, log:\n{log}"
        );
        let count_run = log.matches("run:").count();
        assert!(
            count_run >= 2,
            "expected initial run + revive run, got {count_run}, log:\n{log}"
        );
    }

    #[tokio::test]
    async fn two_concurrent_acquires_get_distinct_slot_indices() {
        let (_base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(DockerPoolConfig {
            runtime_bin: bin_path.to_string_lossy().into_owned(),
            work_root: work,
            pool_size: 2,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("conc".into()),
        })
        .unwrap();
        let p1 = Arc::clone(&pool);
        let p2 = Arc::clone(&pool);
        let (a, b) = tokio::join!(
            p1.acquire_slot(Duration::from_secs(5)),
            p2.acquire_slot(Duration::from_secs(5)),
        );
        let a = a.unwrap();
        let b = b.unwrap();
        assert_ne!(
            a.slot_index, b.slot_index,
            "leased slots must not alias the same pool index"
        );
        DockerPoolManager::release_slot(&pool, a).await.unwrap();
        DockerPoolManager::release_slot(&pool, b).await.unwrap();
    }

    #[tokio::test]
    async fn exec_after_release_is_rejected() {
        let (_base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(DockerPoolConfig {
            runtime_bin: bin_path.to_string_lossy().into_owned(),
            work_root: work,
            pool_size: 1,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("rel".into()),
        })
        .unwrap();
        let lease = pool.acquire_slot(Duration::from_secs(5)).await.unwrap();
        DockerPoolManager::release_slot(&pool, lease.clone())
            .await
            .unwrap();
        let err = pool
            .exec_solve(&lease, "t.json", 1, "claw")
            .await
            .expect_err("exec on released lease must fail");
        assert!(err.contains("invalid or released"), "unexpected err: {err}");
    }

    #[tokio::test]
    async fn double_release_is_idempotent() {
        let (_base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(DockerPoolConfig {
            runtime_bin: bin_path.to_string_lossy().into_owned(),
            work_root: work,
            pool_size: 1,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("dbl".into()),
        })
        .unwrap();
        let lease = pool.acquire_slot(Duration::from_secs(5)).await.unwrap();
        DockerPoolManager::release_slot(&pool, lease.clone())
            .await
            .unwrap();
        DockerPoolManager::release_slot(&pool, lease).await.unwrap();
    }
}
