//! Docker/Podman worker pool: env read once at construction; internal `ensure_warm`. Author: kejiqing

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tracing::{info, warn};
use uuid::Uuid;

use super::config::DockerPoolConfig;
use super::docker_cli::{runtime_exec, runtime_exec_with_live_stderr};
use super::traits::{PoolSessionHostMounts, SlotLease, TaskOutcome};

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

/// Pool of long-lived worker containers.
///
/// Each **lease** rebinds the worker with `docker run -v <session_dir>:GUEST_WORK_ROOT` so the
/// container only sees one solve session directory (no sibling sessions / other `ds_*`).
/// Optional extra binds: `ds_*/home/skills` → `.../home/skills:ro`, `ds_*/CLAUDE.md` → `.../CLAUDE.md:ro` (no per-session copy).
/// Idle warm slots use [`DockerPoolManager::warm_slot_dir`] under `work_root` (empty until lease).
/// Author: kejiqing
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
    /// Optional `sh -lc` body run inside the worker when a lease is returned to idle.
    on_release_exec: Option<String>,
    /// `docker exec --user` for solve only (see [`DockerPoolConfig::exec_user`]).
    exec_user: Option<String>,
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
            on_release_exec: cfg.on_release_exec,
            exec_user: cfg.exec_user,
        }))
    }

    /// Read `CLAW_DOCKER_POOL_*` or `CLAW_PODMAN_POOL_*` once at construction.
    pub fn try_from_env(podman: bool, work_root: &Path) -> Result<Arc<Self>, String> {
        let (default_bin, pfx) = if podman {
            ("podman", "CLAW_PODMAN_")
        } else {
            ("docker", "CLAW_DOCKER_")
        };
        let mut pool_size = std::env::var(format!("{pfx}POOL_SIZE"))
            .unwrap_or_else(|_| "2".to_string())
            .parse::<usize>()
            .map_err(|_| format!("{pfx}POOL_SIZE must be a positive integer"))?;
        if let Ok(cap_s) = std::env::var("CLAW_POOL_SIZE_CAP") {
            if let Ok(cap) = cap_s.trim().parse::<usize>() {
                if cap > 0 && pool_size > cap {
                    warn!(
                        target: "claw_gateway_pool",
                        component = "docker_pool",
                        phase = "pool_size_capped",
                        requested = pool_size,
                        cap,
                        "CLAW_POOL_SIZE_CAP applied"
                    );
                    pool_size = cap;
                }
            }
        }
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
        let mut extra_run_args = std::env::var(format!("{pfx}EXTRA_ARGS"))
            .map(|s| s.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            .unwrap_or_default();
        if let Ok(cpus) = std::env::var(format!("{pfx}POOL_CPUS")) {
            let t = cpus.trim();
            if !t.is_empty() {
                extra_run_args.push("--cpus".into());
                extra_run_args.push(t.to_string());
            }
        }
        if let Ok(mem) = std::env::var(format!("{pfx}POOL_MEMORY")) {
            let t = mem.trim();
            if !t.is_empty() {
                extra_run_args.push("--memory".into());
                extra_run_args.push(t.to_string());
            }
        }
        let on_release_exec = std::env::var(format!("{pfx}POOL_ON_RELEASE"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let exec_user = std::env::var(format!("{pfx}POOL_EXEC_USER"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Self::from_config(DockerPoolConfig {
            runtime_bin: default_bin.to_string(),
            work_root: work_root.to_path_buf(),
            pool_size,
            min_idle,
            image,
            network_args,
            extra_run_args,
            name_stem: None,
            on_release_exec,
            exec_user,
        })
    }

    /// Test hook: run the same warm pass as `schedule_warm` (revive `Dead`, fill `min_idle`).
    #[cfg(test)]
    pub async fn test_ensure_warm_now(self: &Arc<Self>) -> Result<(), String> {
        self.ensure_warm().await
    }

    fn container_name(&self, idx: usize) -> String {
        format!("claw-worker-{}-{idx}", self.name_stem)
    }

    fn exec_solve_argv_prefix(&self) -> Vec<String> {
        let mut v = vec!["exec".to_string()];
        if let Some(ref u) = self.exec_user {
            let t = u.trim();
            if !t.is_empty() {
                v.push("--user".to_string());
                v.push(t.to_string());
            }
        }
        v
    }

    pub fn schedule_warm(self: &Arc<Self>) {
        let s = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = s.ensure_warm().await {
                warn!(
                    target: "claw_gateway_pool",
                    component = "docker_pool",
                    phase = "ensure_warm_failed",
                    error = %e,
                    "pool ensure_warm failed"
                );
            }
        });
    }

    fn warm_slot_dir(&self, idx: usize) -> PathBuf {
        self.work_root_host
            .join(".claw-gateway-pool-warm")
            .join(format!("slot-{idx}"))
    }

    async fn ensure_warm(self: &Arc<Self>) -> Result<(), String> {
        let _ =
            tokio::fs::create_dir_all(self.work_root_host.join(".claw-gateway-pool-warm")).await;
        let mut slots = self.slots.lock().await;
        for i in 0..slots.len() {
            if slots[i].state != SlotState::Dead {
                continue;
            }
            let old = slots[i].container_name.clone();
            drop(slots);
            let _ = self.rm_container(&old).await;
            let name = self.container_name(i);
            let warm = self.warm_slot_dir(i);
            let _ = tokio::fs::create_dir_all(&warm).await;
            let empty_mounts = PoolSessionHostMounts::default();
            self.run_worker_container(&name, &warm, &empty_mounts)
                .await?;
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
            let warm = self.warm_slot_dir(idx);
            let _ = tokio::fs::create_dir_all(&warm).await;
            let empty_mounts = PoolSessionHostMounts::default();
            self.run_worker_container(&name, &warm, &empty_mounts)
                .await?;
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

    async fn run_worker_container(
        &self,
        name: &str,
        session_host_bind: &Path,
        host_mounts: &PoolSessionHostMounts,
    ) -> Result<(), String> {
        let session_abs = std::fs::canonicalize(session_host_bind).map_err(|e| {
            format!(
                "canonicalize pool bind mount {}: {e}",
                session_host_bind.display()
            )
        })?;
        let skills_abs =
            if let Some(p) = host_mounts.skills_dir.as_ref() {
                if std::fs::metadata(p).is_ok_and(|m| m.is_dir()) {
                    Some(std::fs::canonicalize(p).map_err(|e| {
                        format!("canonicalize skills bind mount {}: {e}", p.display())
                    })?)
                } else {
                    None
                }
            } else {
                None
            };
        let claude_abs = if let Some(p) = host_mounts.claude_md_file.as_ref() {
            if std::fs::metadata(p).is_ok_and(|m| m.is_file()) {
                Some(std::fs::canonicalize(p).map_err(|e| {
                    format!("canonicalize CLAUDE.md bind mount {}: {e}", p.display())
                })?)
            } else {
                None
            }
        } else {
            None
        };
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
        args.push(format!("{}:{}:rw", session_abs.display(), GUEST_WORK_ROOT));
        if let Some(ref sk) = skills_abs {
            args.push("-v".into());
            args.push(format!(
                "{}:{}/home/skills:ro",
                sk.display(),
                GUEST_WORK_ROOT
            ));
        }
        if let Some(ref cl) = claude_abs {
            args.push("-v".into());
            args.push(format!("{}:{}/CLAUDE.md:ro", cl.display(), GUEST_WORK_ROOT));
        }
        args.push(self.image.clone());
        args.push("sleep".into());
        args.push("infinity".into());
        let exec_argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = runtime_exec(&self.bin, &exec_argv)
            .await
            .map_err(|e| format!("spawn {}: {e}", self.bin))?;
        if !out.status.success() {
            warn!(
                target: "claw_gateway_pool",
                component = "docker_pool",
                phase = "worker_run_failed",
                container = %name,
                bind_mount = %session_abs.display(),
                code = ?out.status.code(),
                stderr = %String::from_utf8_lossy(&out.stderr).chars().take(2000).collect::<String>(),
                "{} run worker failed",
                self.bin
            );
            return Err(format!(
                "{} run failed: {}",
                self.bin,
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        info!(
            target: "claw_gateway_pool",
            component = "docker_pool",
            phase = "worker_run_ok",
            container = %name,
            bind_mount = %session_abs.display(),
            skills_bind = %skills_abs.as_ref().map_or_else(|| "-".into(), |p| p.display().to_string()),
            claude_bind = %claude_abs.as_ref().map_or_else(|| "-".into(), |p| p.display().to_string()),
            image = %self.image,
            "{} run worker ok",
            self.bin
        );
        Ok(())
    }

    async fn rm_container(&self, name: &str) -> Result<(), String> {
        let _ = runtime_exec(&self.bin, &["rm", "-f", name]).await;
        Ok(())
    }

    /// `session_host_mount` must be an existing directory on the host (typically
    /// `…/ds_{id}/sessions/{uuid}/`); it is canonicalized and bound to [`GUEST_WORK_ROOT`] for this
    /// lease (replacing any warm-slot bind).
    #[allow(clippy::too_many_lines)]
    pub async fn acquire_slot(
        self: &Arc<Self>,
        wait: Duration,
        session_host_mount: PathBuf,
        host_mounts: PoolSessionHostMounts,
    ) -> Result<SlotLease, String> {
        let session_abs = std::fs::canonicalize(&session_host_mount).map_err(|e| {
            format!(
                "canonicalize session bind {}: {e}",
                session_host_mount.display()
            )
        })?;
        let host_mounts = host_mounts.clone();
        timeout(wait, async move {
            loop {
                let mut slots = self.slots.lock().await;
                if let Some((i, _)) = slots
                    .iter()
                    .enumerate()
                    .find(|(_, s)| s.state == SlotState::Idle)
                {
                    let cname = slots[i].container_name.clone();
                    // Reserve before `rm`/`run` so a concurrent acquire cannot pick the same idle slot.
                    slots[i].state = SlotState::Leased;
                    drop(slots);
                    let _ = self.rm_container(&cname).await;
                    if let Err(e) = self
                        .run_worker_container(&cname, &session_abs, &host_mounts)
                        .await
                    {
                        warn!(
                            target: "claw_gateway_pool",
                            component = "docker_pool",
                            phase = "rebind_worker_failed",
                            slot_index = i,
                            error = %e,
                            "pool rebind worker for session mount failed"
                        );
                        let mut slots = self.slots.lock().await;
                        if let Some(s) = slots.get_mut(i) {
                            s.state = SlotState::Dead;
                        }
                        drop(slots);
                        sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                    info!(
                        target: "claw_gateway_pool",
                        component = "docker_pool",
                        phase = "acquire_slot_ok",
                        slot_index = i,
                        session_bind = %session_abs.display(),
                        container = %cname,
                        "reused idle slot; worker rebound to session directory"
                    );
                    return Ok(SlotLease { slot_index: i });
                }
                let total = slots.len();
                if total < self.pool_size {
                    let idx = total;
                    let name = self.container_name(idx);
                    // Reserve slot index before `run` so concurrent acquires cannot claim the same idx.
                    slots.push(Slot {
                        container_name: name.clone(),
                        state: SlotState::Leased,
                    });
                    drop(slots);
                    match self
                        .run_worker_container(&name, &session_abs, &host_mounts)
                        .await
                    {
                        Ok(()) => {
                            info!(
                                target: "claw_gateway_pool",
                                component = "docker_pool",
                                phase = "acquire_slot_ok",
                                slot_index = idx,
                                session_bind = %session_abs.display(),
                                container = %name,
                                "new pool slot created on demand with session bind"
                            );
                            return Ok(SlotLease { slot_index: idx });
                        }
                        Err(e) => {
                            warn!(
                                target: "claw_gateway_pool",
                                component = "docker_pool",
                                phase = "on_demand_worker_create_failed",
                                slot_index = idx,
                                error = %e,
                                "pool on-demand worker create failed"
                            );
                            let mut slots = self.slots.lock().await;
                            if slots.len() == idx + 1 && slots[idx].container_name == name {
                                slots.pop();
                            } else if let Some(s) = slots.get_mut(idx) {
                                s.state = SlotState::Dead;
                            }
                            drop(slots);
                            sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                    }
                }
                drop(slots);
                sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| "acquire_slot: timeout waiting for idle worker".to_string())?
    }

    /// `task_rel_under_root` is a path relative to the session bind root (e.g.
    /// `gateway-solve-task.json`), not under other `ds_*` trees.
    pub async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        request_id: Option<&str>,
    ) -> Result<TaskOutcome, String> {
        let name = {
            let slots = self.slots.lock().await;
            slots
                .get(slot.slot_index)
                .filter(|s| s.state == SlotState::Leased)
                .map(|s| s.container_name.clone())
                .ok_or_else(|| "invalid or released slot".to_string())?
        };
        let container_log = name.clone();
        let workdir = GUEST_WORK_ROOT.to_string();
        let task_path = format!("{GUEST_WORK_ROOT}/{task_rel_under_root}");
        info!(
            target: "claw_gateway_pool",
            component = "docker_pool",
            phase = "exec_solve_start",
            slot_index = slot.slot_index,
            container = %container_log,
            workdir = %workdir,
            task_path = %task_path,
            claw_bin = %claw_bin,
            "docker exec gateway-solve-once starting"
        );
        let mut argv = self.exec_solve_argv_prefix();
        argv.extend(["-e".into(), "CLAW_GATEWAY_WORK_ROOT=/claw_host_root".into()]);
        // Worker `docker run` does not inherit the pool host env; forward MCP tool-call budget so
        // long SQLBot/streamable HTTP calls respect CLAW_MCP_TOOL_CALL_TIMEOUT_MS. Author: kejiqing
        if let Ok(value) = std::env::var("CLAW_MCP_TOOL_CALL_TIMEOUT_MS") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                argv.extend([
                    "-e".into(),
                    format!("CLAW_MCP_TOOL_CALL_TIMEOUT_MS={trimmed}"),
                ]);
            }
        }
        argv.extend([
            "--workdir".into(),
            workdir,
            name,
            claw_bin.to_string(),
            "gateway-solve-once".into(),
            "--task-file".into(),
            task_path,
        ]);
        let argv_refs: Vec<&str> = argv.iter().map(String::as_str).collect();
        let out = runtime_exec_with_live_stderr(&self.bin, &argv_refs, request_id)
            .await
            .map_err(|e| format!("{} exec: {e}", self.bin))?;
        let exit_code = out.status.code().unwrap_or(-1);
        info!(
            target: "claw_gateway_pool",
            component = "docker_pool",
            phase = "exec_solve_done",
            slot_index = slot.slot_index,
            container = %container_log,
            exit_code,
            stdout_len = out.stdout.len(),
            stderr_len = out.stderr.len(),
            "docker exec gateway-solve-once finished"
        );
        Ok(TaskOutcome {
            exit_code,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    pub async fn release_slot(self: &Arc<Self>, slot: SlotLease) -> Result<(), String> {
        let (was_leased, container_name) = {
            let mut slots = self.slots.lock().await;
            let s = slots
                .get_mut(slot.slot_index)
                .ok_or_else(|| "release: bad slot index".to_string())?;
            let was_leased = s.state == SlotState::Leased;
            let name = s.container_name.clone();
            if was_leased {
                s.state = SlotState::Idle;
            }
            (was_leased, name)
        };
        if was_leased {
            if let Some(ref script) = self.on_release_exec {
                if !script.trim().is_empty() {
                    self.run_on_release_hook(&container_name, script).await;
                }
            }
        }
        Self::schedule_warm(self);
        Ok(())
    }

    /// Best-effort cleanup inside the worker after a normal lease return (not on `force_kill`).
    async fn run_on_release_hook(&self, container_name: &str, script: &str) {
        let argv = [
            "exec",
            "-e",
            "CLAW_GATEWAY_WORK_ROOT=/claw_host_root",
            "--workdir",
            "/",
            container_name,
            "sh",
            "-lc",
            script,
        ];
        match runtime_exec(&self.bin, &argv).await {
            Ok(out) if out.status.success() => {
                tracing::debug!(
                    target: "claw_gateway_pool",
                    component = "docker_pool",
                    phase = "on_release_ok",
                    container = %container_name,
                    "pool POOL_ON_RELEASE hook finished"
                );
            }
            Ok(out) => {
                warn!(
                    target: "claw_gateway_pool",
                    component = "docker_pool",
                    phase = "on_release_nonzero",
                    container = %container_name,
                    code = ?out.status.code(),
                    stderr = %String::from_utf8_lossy(&out.stderr),
                    "pool POOL_ON_RELEASE hook exited non-zero"
                );
            }
            Err(e) => {
                warn!(
                    target: "claw_gateway_pool",
                    component = "docker_pool",
                    phase = "on_release_spawn_failed",
                    container = %container_name,
                    error = %e,
                    "pool POOL_ON_RELEASE hook spawn failed"
                );
            }
        }
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
        // Propagate a cooperative stop to the worker first; `sleep infinity` exits on SIGTERM,
        // which tears down in-flight `docker exec` sessions. Follow with SIGKILL for stubborn
        // containers so the pool slot can be revived.
        let _ = runtime_exec(&self.bin, &["kill", "-s", "SIGTERM", name.as_str()]).await;
        sleep(Duration::from_millis(400)).await;
        let _ = runtime_exec(&self.bin, &["kill", "-s", "SIGKILL", name.as_str()]).await;
        Self::schedule_warm(self);
        Ok(())
    }
}

#[cfg(test)]
impl DockerPoolManager {
    pub(crate) fn test_exec_solve_argv_prefix(&self) -> Vec<String> {
        self.exec_solve_argv_prefix()
    }
}

#[cfg(test)]
mod exec_solve_argv_prefix_tests {
    use std::sync::Arc;

    use super::DockerPoolManager;
    use crate::pool::config::DockerPoolConfig;

    fn pool(exec_user: Option<&str>) -> Arc<DockerPoolManager> {
        let base =
            std::env::temp_dir().join(format!("gw-exec-prefix-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&base).unwrap();
        DockerPoolManager::from_config(DockerPoolConfig {
            runtime_bin: "docker".into(),
            work_root: base,
            pool_size: 1,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("pfxtest".into()),
            on_release_exec: None,
            exec_user: exec_user.map(str::to_string),
        })
        .expect("from_config")
    }

    #[test]
    fn exec_prefix_omits_user_when_unset() {
        let p = pool(None);
        assert_eq!(p.test_exec_solve_argv_prefix(), vec!["exec".to_string()]);
    }

    #[test]
    fn exec_prefix_includes_trimmed_user() {
        let p = pool(Some("  claw  "));
        assert_eq!(
            p.test_exec_solve_argv_prefix(),
            vec!["exec".to_string(), "--user".to_string(), "claw".to_string()]
        );
    }

    #[test]
    fn exec_prefix_skips_whitespace_only_user() {
        let p = pool(Some("   \t  "));
        assert_eq!(p.test_exec_solve_argv_prefix(), vec!["exec".to_string()]);
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
    use crate::pool::traits::PoolSessionHostMounts;
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

    fn session_bind(work: &Path) -> PathBuf {
        let d = work.join(format!("sess-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&d).unwrap();
        fs::canonicalize(&d).unwrap()
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
            on_release_exec: None,
            exec_user: None,
        })
        .unwrap();
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        let out = pool
            .exec_solve(&lease, "gateway-solve-task.json", "claw", None)
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
            work_root: work.clone(),
            pool_size: 2,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("killme".into()),
            on_release_exec: None,
            exec_user: None,
        })
        .unwrap();
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
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
            work_root: work.clone(),
            pool_size: 2,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("conc".into()),
            on_release_exec: None,
            exec_user: None,
        })
        .unwrap();
        let p1 = Arc::clone(&pool);
        let p2 = Arc::clone(&pool);
        let b1 = session_bind(&work);
        let b2 = session_bind(&work);
        let (a, b) = tokio::join!(
            p1.acquire_slot(Duration::from_secs(5), b1, PoolSessionHostMounts::default()),
            p2.acquire_slot(Duration::from_secs(5), b2, PoolSessionHostMounts::default()),
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
            work_root: work.clone(),
            pool_size: 1,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("rel".into()),
            on_release_exec: None,
            exec_user: None,
        })
        .unwrap();
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        DockerPoolManager::release_slot(&pool, lease.clone())
            .await
            .unwrap();
        let err = pool
            .exec_solve(&lease, "gateway-solve-task.json", "claw", None)
            .await
            .expect_err("exec on released lease must fail");
        assert!(err.contains("invalid or released"), "unexpected err: {err}");
    }

    #[tokio::test]
    async fn double_release_is_idempotent() {
        let (_base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(DockerPoolConfig {
            runtime_bin: bin_path.to_string_lossy().into_owned(),
            work_root: work.clone(),
            pool_size: 1,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("dbl".into()),
            on_release_exec: None,
            exec_user: None,
        })
        .unwrap();
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        DockerPoolManager::release_slot(&pool, lease.clone())
            .await
            .unwrap();
        DockerPoolManager::release_slot(&pool, lease).await.unwrap();
    }

    #[tokio::test]
    async fn release_runs_configured_on_release_hook() {
        let (base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(DockerPoolConfig {
            runtime_bin: bin_path.to_string_lossy().into_owned(),
            work_root: work.clone(),
            pool_size: 1,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("relhook".into()),
            on_release_exec: Some("echo pool_on_release".into()),
            exec_user: None,
        })
        .unwrap();
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        pool.exec_solve(&lease, "gateway-solve-task.json", "claw", None)
            .await
            .unwrap();
        DockerPoolManager::release_slot(&pool, lease).await.unwrap();
        let log = read_log(&state_dir);
        let exec_lines: Vec<&str> = log.lines().filter(|l| l.starts_with("exec:")).collect();
        assert!(
            exec_lines.len() >= 2,
            "expected solve exec + release hook exec, log:\n{log}"
        );
        assert!(
            log.contains("pool_on_release"),
            "release hook should run sh -lc script, log:\n{log}"
        );
    }

    #[tokio::test]
    async fn exec_solve_includes_user_when_configured() {
        let (base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(DockerPoolConfig {
            runtime_bin: bin_path.to_string_lossy().into_owned(),
            work_root: work.clone(),
            pool_size: 1,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some("uidtest".into()),
            on_release_exec: None,
            exec_user: Some("claw".into()),
        })
        .unwrap();
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        pool.exec_solve(&lease, "gateway-solve-task.json", "claw", None)
            .await
            .unwrap();
        let log = read_log(&state_dir);
        assert!(
            log.contains("--user") && log.contains("claw"),
            "solve exec should pass --user claw, log:\n{log}"
        );
    }
}
