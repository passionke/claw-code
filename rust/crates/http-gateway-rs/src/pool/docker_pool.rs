//! Docker/Podman worker pool: env read once at construction; internal `ensure_warm`. Author: kejiqing

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tracing::{info, warn};
use uuid::Uuid;

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
    /// Read `CLAW_DOCKER_POOL_*` or `CLAW_PODMAN_POOL_*` once at construction.
    pub fn try_from_env(podman: bool, work_root: &Path) -> Result<Arc<Self>, String> {
        let (bin, pfx) = if podman {
            ("podman", "CLAW_PODMAN_")
        } else {
            ("docker", "CLAW_DOCKER_")
        };
        let pool_size = std::env::var(format!("{pfx}POOL_SIZE"))
            .unwrap_or_else(|_| "2".to_string())
            .parse::<usize>()
            .map_err(|_| format!("{pfx}POOL_SIZE must be a positive integer"))?;
        if pool_size == 0 {
            return Err(format!("{pfx}POOL_SIZE must be >= 1"));
        }
        let min_idle = std::env::var(format!("{pfx}POOL_MIN_IDLE"))
            .unwrap_or_else(|_| "1".to_string())
            .parse::<usize>()
            .map_err(|_| format!("{pfx}POOL_MIN_IDLE must be an integer"))?;
        if min_idle > pool_size {
            return Err(format!(
                "{pfx}POOL_MIN_IDLE ({min_idle}) must be <= {pfx}POOL_SIZE ({pool_size})"
            ));
        }
        let image = std::env::var(format!("{pfx}IMAGE"))
            .map_err(|_| format!("{pfx}IMAGE is required for container pool"))?;
        if image.trim().is_empty() {
            return Err(format!("{pfx}IMAGE must be non-empty"));
        }
        let network_args = match std::env::var(format!("{pfx}NETWORK")) {
            Ok(n) if !n.trim().is_empty() => vec!["--network".to_string(), n.trim().to_string()],
            _ => Vec::new(),
        };
        let extra_run_args = std::env::var(format!("{pfx}EXTRA_ARGS"))
            .map(|s| s.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        let work_root_host = std::fs::canonicalize(work_root)
            .map_err(|e| format!("canonicalize work_root {}: {e}", work_root.display()))?;
        let name_stem = Uuid::new_v4().simple().to_string();
        let name_stem = name_stem[..8].to_string();
        Ok(Arc::new(Self {
            name_stem,
            bin: bin.to_string(),
            slots: Mutex::new(Vec::new()),
            work_root_host,
            pool_size,
            min_idle,
            image,
            network_args,
            extra_run_args,
        }))
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
