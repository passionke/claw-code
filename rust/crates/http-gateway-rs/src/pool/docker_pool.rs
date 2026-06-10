//! Docker/Podman worker pool: env read once at construction; internal `ensure_warm`. Author: kejiqing

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tracing::{info, warn};
use uuid::Uuid;

use gateway_solve_turn::WORKER_ENV_MOUNT_PATH;

use super::config::{
    fixed_isolation_from_env, relaxed_worker_allowed_from_env, security_boost_from_env,
    DockerPoolConfig,
};
use super::docker_cli::{
    probe_container_runtime_cli, runtime_exec, runtime_exec_with_live_streams,
};
use super::session_db_sync::{self, MaterializeInput};
use super::traits::{SlotLease, TaskOutcome};
use super::worker_identity::PoolWorkerIdentity;
use super::worker_isolation::{effective_mode, exec_user_arg_for_mode, WorkerIsolationMode};

pub const GUEST_WORK_ROOT: &str = "/claw_host_root";

/// Base stem budget when no `-strict` / `-relaxed` profile suffix is present.
const WORKER_NAME_STEM_BASE_MAX: usize = 16;

/// Build `claw-worker-{stem}-{n}` stem from pool id suffix (after optional `pool-` strip).
/// Reserves room for `-strict` / `-relaxed` profile suffixes on worker names. Author: kejiqing
fn worker_name_stem_from_pool_suffix(suffix: &str) -> String {
    let (base, profile) = if let Some(b) = suffix.strip_suffix("-strict") {
        (b, Some("strict"))
    } else if let Some(b) = suffix.strip_suffix("-relaxed") {
        (b, Some("relaxed"))
    } else {
        (suffix, None)
    };

    let profile_chars = profile.map(|p| p.len() + 1).unwrap_or(0);
    let base_max = WORKER_NAME_STEM_BASE_MAX
        .saturating_sub(profile_chars)
        .max(1);
    let mut base_stem: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(base_max)
        .collect();
    while base_stem.ends_with('-') {
        base_stem.pop();
    }
    if base_stem.is_empty() {
        let u = Uuid::new_v4().simple().to_string();
        base_stem = u[..8].to_string();
    }
    match profile {
        Some(p) => format!("{base_stem}-{p}"),
        None => base_stem,
    }
}

/// Stable `claw-worker-{stem}-{n}` prefix from `CLAW_POOL_ID` (avoid orphan containers per restart). Author: kejiqing
fn default_worker_name_stem() -> String {
    if let Ok(raw) = std::env::var("CLAW_POOL_ID") {
        let id = raw.trim();
        if !id.is_empty() {
            let suffix = id.strip_prefix("pool-").unwrap_or(id);
            return worker_name_stem_from_pool_suffix(suffix);
        }
    }
    let u = Uuid::new_v4().simple().to_string();
    u[..8].to_string()
}

/// Host path bind-mounted to [`WORKER_ENV_MOUNT_PATH`] (single file; colon lists are invalid paths).
fn resolve_worker_env_host_file() -> Option<PathBuf> {
    let raw = std::env::var("CLAW_WORKER_ENV_FILE").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed.contains(':') {
        let p = PathBuf::from(trimmed);
        return p.is_file().then_some(p);
    }
    for part in trimmed.split(':').map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(part);
        if p.is_file()
            && (part.ends_with("claw-worker-runtime.env")
                || (Path::new(part)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("env"))
                    && !part.contains("claw-worker-llm")))
        {
            return Some(p);
        }
    }
    for part in trimmed.split(':').map(str::trim).filter(|s| !s.is_empty()) {
        let p = PathBuf::from(part);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Serial FIFO forwarder for one turn's stdout lines.
///
/// Each stdout line from `docker exec` arrives via the synchronous Fn callback below.
/// We CANNOT `.await` inside that callback, so the prior implementation called
/// `tokio::spawn(async { ... HTTP forward ... })` per line. That made N forwards
/// race in parallel: under realistic streaming load the HTTP POSTs arrived at the
/// gateway out of order, so SSE subscribers saw token sequence scrambled.
///
/// Fix: one mpsc channel + one consumer task per turn. The Fn just `send`s the line
/// (lock-free, ordered); the single consumer drains in FIFO order and awaits each
/// forward sequentially. Author: kejiqing
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn merge_stdout_hooks(
    turn_id: &str,
    hub: Option<Arc<super::live_report_hub::LiveReportHub>>,
    outer: Option<Arc<dyn Fn(String) + Send + Sync>>,
) -> Option<Arc<dyn Fn(String) + Send + Sync>> {
    let tid = turn_id.to_string();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let tid_for_worker = tid.clone();
    let hub_for_worker = hub.clone();
    let outer_for_worker = outer.clone();
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if let Some(ref o) = outer_for_worker {
                o(line.clone());
            }
            if let Some(ref h) = hub_for_worker {
                h.ingest_stdout_line(&tid_for_worker, &line);
            }
        }
    });
    let hook: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |line: String| {
        let _ = tx.send(line);
    });
    Some(hook)
}

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
    /// `proj_id` bound at last `run` (revive when changed). Author: kejiqing
    bound_proj_id: Option<i64>,
    /// Worker strict/relaxed profile bound at last `run`. Author: kejiqing
    bound_isolation: WorkerIsolationMode,
    /// Integration tests: host session tree bind-mounted to [`GUEST_WORK_ROOT`].
    test_host_root: Option<PathBuf>,
}

/// Symlink inject only for the in-process `fake-docker` unit-test shim (no host `mount(8)`).
/// Production v1: tmpfs `/claw_host_root` + PG `materialize_in` / `readback_out` (no session bind).
fn use_symlink_inject(runtime_bin: &str) -> bool {
    runtime_bin.contains("fake-docker")
}

/// Pool of long-lived worker containers (Phase 2).
///
/// Each slot `run`s with `proj_{id}` → `/claw_ds` and ephemeral [`GUEST_WORK_ROOT`] (tmpfs).
/// **Acquire** materializes session files from PG via `docker exec`; **release** only pkill. Author: kejiqing
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
    on_release_exec: Option<String>,
    worker_identity: PoolWorkerIdentity,
    security_boost: bool,
    fixed_isolation: Option<WorkerIsolationMode>,
    symlink_inject: bool,
    worker_env_host_file: Option<PathBuf>,
    live_report_hub: Option<Arc<super::live_report_hub::LiveReportHub>>,
    pool_id: Option<String>,
    session_db: Option<Arc<crate::session_db::GatewaySessionDb>>,
}

impl DockerPoolManager {
    /// Build pool from explicit config (tests inject a fake `docker` binary path).
    pub fn from_config(cfg: DockerPoolConfig) -> Result<Arc<Self>, String> {
        cfg.validate()?;
        let work_root_host = std::fs::canonicalize(&cfg.work_root)
            .map_err(|e| format!("canonicalize work_root {}: {e}", cfg.work_root.display()))?;
        let name_stem = cfg.name_stem.unwrap_or_else(default_worker_name_stem);
        Ok(Arc::new(Self {
            name_stem,
            bin: cfg.runtime_bin.clone(),
            slots: Mutex::new(Vec::new()),
            work_root_host,
            pool_size: cfg.pool_size,
            min_idle: cfg.min_idle,
            image: cfg.image,
            network_args: cfg.network_args,
            extra_run_args: cfg.extra_run_args,
            on_release_exec: cfg.on_release_exec,
            worker_identity: cfg.worker_identity,
            security_boost: cfg.security_boost,
            fixed_isolation: cfg.fixed_isolation,
            symlink_inject: cfg.symlink_inject,
            worker_env_host_file: cfg.worker_env_host_file,
            live_report_hub: cfg.live_report_hub,
            pool_id: cfg.pool_id,
            session_db: cfg.session_db,
        }))
    }

    #[must_use]
    pub fn live_report_hub(&self) -> Option<Arc<super::live_report_hub::LiveReportHub>> {
        self.live_report_hub.clone()
    }

    #[must_use]
    pub fn has_report_for_turn(&self, turn_id: &str) -> bool {
        self.live_report_hub
            .as_ref()
            .is_some_and(|hub| hub.has_report_for_turn(turn_id))
    }

    #[must_use]
    pub fn first_report_at_ms_for_turn(&self, turn_id: &str) -> Option<i64> {
        self.live_report_hub
            .as_ref()
            .and_then(|hub| hub.first_report_at_ms_for_turn(turn_id))
    }

    #[must_use]
    pub fn slot_capacity(&self) -> (usize, usize) {
        (self.pool_size, self.min_idle)
    }

    /// Running turn: worker tmpfs `.claw` → `gateway_turns.solve_timing_jsonb` (host podman only). Author: kejiqing
    pub async fn sync_turn_progress_to_db(&self, turn_id: &str) -> Result<(), String> {
        let db = self
            .session_db
            .as_ref()
            .ok_or_else(|| "pool session_db unavailable".to_string())?;
        let Some(worker) = db
            .get_turn_worker_name(turn_id)
            .await
            .map_err(|e| format!("get worker_name: {e}"))?
        else {
            return Ok(());
        };
        if worker.trim().is_empty() {
            return Ok(());
        }
        let (progress_ndjson, task_progress_json) =
            session_db_sync::read_worker_progress_artifacts(&self.bin, &worker).await;
        if progress_ndjson.is_empty() && task_progress_json.is_empty() {
            return Ok(());
        }
        db.replace_turn_progress_snapshot(turn_id, &progress_ndjson, &task_progress_json)
            .await
            .map_err(|e| format!("replace turn progress: {e}"))?;
        Ok(())
    }

    /// Read `CLAW_DOCKER_POOL_*` or `CLAW_PODMAN_POOL_*` once at construction.
    #[allow(clippy::too_many_lines)]
    pub fn try_from_env(
        podman: bool,
        work_root: &Path,
        live_report_hub: Option<Arc<super::live_report_hub::LiveReportHub>>,
        registry: Option<(String, Arc<crate::session_db::GatewaySessionDb>)>,
    ) -> Result<Arc<Self>, String> {
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
        let pool_network = std::env::var(format!("{pfx}NETWORK"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let network_args = pool_network
            .as_ref()
            .map(|n| vec!["--network".to_string(), n.clone()])
            .unwrap_or_default();
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
        let worker_env_host_file = resolve_worker_env_host_file();
        let runtime_bin = default_bin.to_string();
        if !podman {
            probe_container_runtime_cli(&runtime_bin)?;
        }
        let worker_identity = PoolWorkerIdentity::from_env(exec_user.clone());
        Self::from_config(DockerPoolConfig {
            runtime_bin: runtime_bin.clone(),
            work_root: work_root.to_path_buf(),
            pool_size,
            min_idle,
            image,
            network_args,
            extra_run_args,
            name_stem: None,
            on_release_exec,
            exec_user,
            worker_identity,
            security_boost: security_boost_from_env(),
            fixed_isolation: fixed_isolation_from_env(),
            symlink_inject: use_symlink_inject(&runtime_bin),
            worker_env_host_file,
            live_report_hub,
            pool_id: registry.as_ref().map(|(id, _)| id.clone()),
            session_db: registry.map(|(_, db)| db),
        })
    }

    /// Test hook: run the same warm pass as `schedule_warm` (revive `Dead`, fill `min_idle`).
    #[cfg(test)]
    pub async fn test_ensure_warm_now(self: &Arc<Self>) -> Result<(), String> {
        self.ensure_warm().await
    }

    fn lease_from_slot(slots: &[Slot], slot_index: usize) -> Result<SlotLease, String> {
        let _ = slots
            .get(slot_index)
            .ok_or_else(|| format!("invalid slot index {slot_index}"))?;
        Ok(SlotLease { slot_index })
    }

    fn container_name(&self, idx: usize) -> String {
        format!("claw-worker-{}-{idx}", self.name_stem)
    }

    fn exec_solve_argv_prefix_for(&self, isolation: WorkerIsolationMode) -> Vec<String> {
        vec![
            "exec".to_string(),
            "--user".to_string(),
            exec_user_arg_for_mode(isolation, &self.worker_identity.exec_user_arg()),
        ]
    }

    async fn resolve_isolation_for_proj(&self, proj_id: i64) -> WorkerIsolationMode {
        if let Some(fixed) = self.fixed_isolation {
            return fixed;
        }
        let json = if let Some(ref db) = self.session_db {
            db.get_worker_isolation_json(proj_id)
                .await
                .unwrap_or_else(|_| super::worker_isolation::default_worker_isolation_json())
        } else {
            super::worker_isolation::default_worker_isolation_json()
        };
        effective_mode(relaxed_worker_allowed_from_env(), &json)
    }

    fn proj_host_dir(&self, proj_id: i64) -> PathBuf {
        self.work_root_host.join(format!("proj_{proj_id}"))
    }

    /// proj_{id} host bind at [`session_db_sync::DS_MOUNT_TARGET`]; read-only in worker (gateway writes on host).
    #[must_use]
    fn proj_home_bind_volume_arg(ds_host_abs: &Path) -> String {
        format!(
            "{}:{}:ro",
            ds_host_abs.display(),
            session_db_sync::DS_MOUNT_TARGET
        )
    }

    #[must_use]
    fn guest_work_root_tmpfs_arg() -> String {
        format!("{GUEST_WORK_ROOT}:rw,size=512m,mode=1777")
    }

    fn append_security_boost_run_args(&self, args: &mut Vec<String>) {
        if !self.security_boost {
            return;
        }
        args.push("--security-opt".into());
        args.push("no-new-privileges".into());
        args.push("--cap-drop".into());
        args.push("ALL".into());
        args.push("--read-only".into());
        args.push("--tmpfs".into());
        args.push("/tmp:rw,noexec,nosuid,size=64m".into());
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

    fn warm_isolation(&self) -> WorkerIsolationMode {
        self.fixed_isolation.unwrap_or(WorkerIsolationMode::Strict)
    }

    async fn ensure_warm(self: &Arc<Self>) -> Result<(), String> {
        let _ = tokio::fs::create_dir_all(self.work_root_host.join(".claw-pool-slot")).await;
        let mut slots = self.slots.lock().await;
        for i in 0..slots.len() {
            if slots[i].state != SlotState::Dead {
                continue;
            }
            let old = slots[i].container_name.clone();
            drop(slots);
            let name = self.container_name(i);
            if old != name {
                let _ = self.rm_container(&old).await;
            }
            self.run_worker_slot_container(i, &name, 1, self.warm_isolation(), None)
                .await?;
            slots = self.slots.lock().await;
            slots[i] = Slot {
                container_name: name,
                state: SlotState::Idle,
                bound_proj_id: Some(1),
                bound_isolation: self.warm_isolation(),
                test_host_root: None,
            };
        }
        let mut idle = slots.iter().filter(|s| s.state == SlotState::Idle).count();
        let mut total = slots.len();
        while idle < self.min_idle && total < self.pool_size {
            let idx = total;
            let name = self.container_name(idx);
            slots.push(Slot {
                container_name: name.clone(),
                state: SlotState::Idle,
                bound_proj_id: None,
                bound_isolation: WorkerIsolationMode::Strict,
                test_host_root: None,
            });
            drop(slots);
            self.run_worker_slot_container(idx, &name, 1, self.warm_isolation(), None)
                .await?;
            slots = self.slots.lock().await;
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

    /// Create or revive a slot: `proj_{id}` → `/claw_ds`, ephemeral [`GUEST_WORK_ROOT`]. Author: kejiqing
    async fn run_worker_slot_container(
        &self,
        slot_index: usize,
        name: &str,
        proj_id: i64,
        isolation: WorkerIsolationMode,
        test_host_root: Option<PathBuf>,
    ) -> Result<(), String> {
        if self.worker_container_running(name).await {
            if self.worker_container_image_stale(name).await {
                info!(
                    target: "claw_gateway_pool",
                    component = "docker_pool",
                    phase = "worker_image_stale",
                    container = %name,
                    slot_index,
                    image = %self.image,
                    "worker image stale — recreating container"
                );
                self.rm_container(name).await?;
            } else if !self.worker_container_bind_matches_proj(name, proj_id).await {
                let want = self.proj_host_dir(proj_id);
                let got = self.worker_container_claw_ds_bind_source(name).await;
                info!(
                    target: "claw_gateway_pool",
                    component = "docker_pool",
                    phase = "worker_bind_stale",
                    container = %name,
                    slot_index,
                    proj_id,
                    want_bind = %want.display(),
                    got_bind = ?got,
                    "worker /claw_ds bind stale — recreating container"
                );
                self.rm_container(name).await?;
            } else {
                info!(
                    target: "claw_gateway_pool",
                    component = "docker_pool",
                    phase = "worker_reuse",
                    container = %name,
                    slot_index,
                    "reusing existing worker container (stable name)"
                );
                // Pool restart or missed release must not leave gateway-solve-once running in the worker.
                self.kill_worker_solve_processes(name, isolation).await;
                return Ok(());
            }
        }
        self.rm_container(name).await?;

        let ds_host = self.proj_host_dir(proj_id);
        tokio::fs::create_dir_all(&ds_host)
            .await
            .map_err(|e| format!("mkdir ds home {}: {e}", ds_host.display()))?;
        let ds_abs = std::fs::canonicalize(&ds_host)
            .map_err(|e| format!("canonicalize ds home {}: {e}", ds_host.display()))?;
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
        if isolation == WorkerIsolationMode::Strict && self.security_boost {
            self.append_security_boost_run_args(&mut args);
        }
        args.push("-v".into());
        args.push(Self::proj_home_bind_volume_arg(&ds_abs));
        if let Some(host_root) = test_host_root.as_ref() {
            let root_abs = std::fs::canonicalize(host_root).map_err(|e| {
                format!(
                    "canonicalize test session root {}: {e}",
                    host_root.display()
                )
            })?;
            args.push("-v".into());
            args.push(format!("{}:{}:rw", root_abs.display(), GUEST_WORK_ROOT));
        } else {
            args.push("--tmpfs".into());
            args.push(Self::guest_work_root_tmpfs_arg());
        }
        if let Some(ref host_env) = self.worker_env_host_file {
            let env_abs = std::fs::canonicalize(host_env).map_err(|e| {
                format!(
                    "canonicalize CLAW_WORKER_ENV_FILE {}: {e}",
                    host_env.display()
                )
            })?;
            args.push("-v".into());
            args.push(format!(
                "{}:{}:ro",
                env_abs.display(),
                WORKER_ENV_MOUNT_PATH
            ));
            args.push("-e".into());
            args.push(format!("CLAW_WORKER_ENV_FILE={WORKER_ENV_MOUNT_PATH}"));
        }
        args.push("--entrypoint".into());
        args.push("sleep".into());
        args.push(self.image.clone());
        args.push("infinity".into());
        let exec_argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut out = runtime_exec(&self.bin, &exec_argv)
            .await
            .map_err(|e| format!("spawn {}: {e}", self.bin))?;
        if !out.status.success() && Self::stderr_name_already_in_use(&out.stderr) {
            info!(
                target: "claw_gateway_pool",
                component = "docker_pool",
                phase = "worker_run_retry",
                container = %name,
                "worker name in use; rm and retry once"
            );
            self.rm_container(name).await?;
            out = runtime_exec(&self.bin, &exec_argv)
                .await
                .map_err(|e| format!("spawn {} retry: {e}", self.bin))?;
        }
        if !out.status.success() {
            warn!(
                target: "claw_gateway_pool",
                component = "docker_pool",
                phase = "worker_run_failed",
                container = %name,
                ds_bind = %ds_abs.display(),
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
            proj_id,
            worker_isolation = ?isolation,
            ds_bind = %ds_abs.display(),
            slot_index,
            image = %self.image,
            "{} run worker slot container ok",
            self.bin
        );
        Ok(())
    }

    async fn kill_worker_solve_processes(
        &self,
        container_name: &str,
        isolation: WorkerIsolationMode,
    ) {
        let script = match isolation {
            WorkerIsolationMode::Relaxed => {
                "pkill -f gateway-solve-once 2>/dev/null || true".to_string()
            }
            WorkerIsolationMode::Strict => {
                let user = self.worker_identity.pkill_user();
                format!("pkill -u {user} -f gateway-solve-once 2>/dev/null || true")
            }
        };
        self.run_on_release_hook(container_name, &script).await;
    }

    async fn rm_container(&self, name: &str) -> Result<(), String> {
        let _ = runtime_exec(&self.bin, &["rm", "-f", name]).await;
        Ok(())
    }

    /// `true` if named container exists and is running (stable stem reuse after pool restart). Author: kejiqing
    async fn worker_container_running(&self, name: &str) -> bool {
        match runtime_exec(&self.bin, &["inspect", "-f", "{{.State.Running}}", name]).await {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim() == "true"
            }
            _ => false,
        }
    }

    async fn container_runtime_image_id(&self, inspect_target: &str) -> Option<String> {
        let out = runtime_exec(&self.bin, &["inspect", "-f", "{{.Image}}", inspect_target])
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if id.is_empty() {
            None
        } else {
            Some(id)
        }
    }

    /// Recreate worker when pool image tag was rebuilt (`pack-deploy`) but container name was reused. Author: kejiqing
    async fn worker_container_image_stale(&self, name: &str) -> bool {
        let Some(running_id) = self.container_runtime_image_id(name).await else {
            return false;
        };
        let Some(desired_id) = self.container_runtime_image_id(&self.image).await else {
            return false;
        };
        running_id != desired_id
    }

    /// Host path currently bind-mounted at `/claw_ds` in a running worker. Author: kejiqing
    async fn worker_container_claw_ds_bind_source(&self, name: &str) -> Option<String> {
        let out = runtime_exec(
            &self.bin,
            &[
                "inspect",
                "-f",
                "{{range .Mounts}}{{if eq .Destination \"/claw_ds\"}}{{.Source}}{{end}}{{end}}",
                name,
            ],
        )
        .await
        .ok()?;
        if !out.status.success() {
            return None;
        }
        let src = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if src.is_empty() {
            None
        } else {
            Some(src)
        }
    }

    fn canonicalize_host_path(path: &Path) -> Option<std::path::PathBuf> {
        std::fs::canonicalize(path).ok()
    }

    /// `true` when worker `/claw_ds` bind matches [`Self::proj_host_dir`] for `proj_id`. Author: kejiqing
    fn worker_bind_matches_proj_host(bind_source: &str, want_host: &Path) -> bool {
        let want = Self::canonicalize_host_path(want_host);
        let got = Self::canonicalize_host_path(Path::new(bind_source));
        match (want, got) {
            (Some(w), Some(g)) => w == g,
            _ => bind_source == want_host.to_string_lossy().as_ref(),
        }
    }

    async fn worker_container_bind_matches_proj(&self, name: &str, proj_id: i64) -> bool {
        let Some(bind_source) = self.worker_container_claw_ds_bind_source(name).await else {
            return false;
        };
        Self::worker_bind_matches_proj_host(&bind_source, &self.proj_host_dir(proj_id))
    }

    /// After external `podman rm` or crash, leased slots may never return to Idle. Author: kejiqing
    async fn reconcile_stale_leased_slots(&self) -> usize {
        let names: Vec<String> = {
            let slots = self.slots.lock().await;
            slots
                .iter()
                .filter(|s| s.state == SlotState::Leased)
                .map(|s| s.container_name.clone())
                .collect()
        };
        let mut freed = 0usize;
        for name in names {
            if self.worker_container_running(&name).await {
                continue;
            }
            let mut slots = self.slots.lock().await;
            let Some(s) = slots.iter_mut().find(|s| s.container_name == name) else {
                continue;
            };
            if s.state != SlotState::Leased {
                continue;
            }
            info!(
                target: "claw_gateway_pool",
                component = "docker_pool",
                phase = "reconcile_stale_lease",
                container = %name,
                "worker gone while slot Leased — returning to Idle"
            );
            s.state = SlotState::Idle;
            freed += 1;
        }
        freed
    }

    fn stderr_name_already_in_use(stderr: &[u8]) -> bool {
        let s = String::from_utf8_lossy(stderr);
        s.contains("already in use") || s.contains("is already in use")
    }

    async fn prepare_slot_for_lease(
        self: &Arc<Self>,
        slot_index: usize,
        proj_id: i64,
        session_id: &str,
        turn_id: &str,
    ) -> Result<String, String> {
        let isolation = self.resolve_isolation_for_proj(proj_id).await;
        let test_host = if self.symlink_inject {
            let home = session_db_sync::session_home_under_work_root(
                &self.work_root_host,
                proj_id,
                session_id,
            );
            tokio::fs::create_dir_all(home.join(".claw"))
                .await
                .map_err(|e| format!("mkdir test session .claw: {e}"))?;
            Some(home)
        } else {
            None
        };
        let exec_user = exec_user_arg_for_mode(isolation, &self.worker_identity.exec_user_arg());
        let (cname, need_run) = {
            let mut slots = self.slots.lock().await;
            let s = slots
                .get_mut(slot_index)
                .ok_or_else(|| "bad slot index".to_string())?;
            let host_changed =
                self.symlink_inject && s.test_host_root.as_ref() != test_host.as_ref();
            let need_run = s.bound_proj_id != Some(proj_id)
                || s.bound_isolation != isolation
                || s.state == SlotState::Dead
                || host_changed;
            s.state = SlotState::Leased;
            s.test_host_root = test_host.clone();
            (s.container_name.clone(), need_run)
        };
        if need_run {
            self.run_worker_slot_container(slot_index, &cname, proj_id, isolation, test_host)
                .await?;
            let mut slots = self.slots.lock().await;
            if let Some(s) = slots.get_mut(slot_index) {
                s.bound_proj_id = Some(proj_id);
                s.bound_isolation = isolation;
            }
        }
        if let Some(ref db) = self.session_db {
            session_db_sync::materialize_in(
                &self.bin,
                &self.work_root_host,
                &cname,
                db,
                &MaterializeInput {
                    session_id: session_id.to_string(),
                    proj_id,
                    turn_id: turn_id.to_string(),
                },
                &exec_user,
            )
            .await?;
        }
        Ok(cname)
    }

    #[allow(clippy::too_many_lines)]
    pub async fn acquire_slot(
        self: &Arc<Self>,
        wait: Duration,
        session_id: String,
        proj_id: i64,
        turn_id: String,
    ) -> Result<SlotLease, String> {
        if let Some(ref db) = self.session_db {
            db.assert_session_can_acquire_for_turn(&session_id, proj_id, &turn_id)
                .await
                .map_err(|reason| format!("session acquire blocked: {reason}"))?;
        }
        timeout(wait, async move {
            let isolation = self.resolve_isolation_for_proj(proj_id).await;
            loop {
                let mut slots = self.slots.lock().await;
                if let Some((i, _)) = slots
                    .iter()
                    .enumerate()
                    .find(|(_, s)| {
                        s.state == SlotState::Idle
                            && s.bound_proj_id == Some(proj_id)
                            && s.bound_isolation == isolation
                    })
                    .or_else(|| {
                        slots
                            .iter()
                            .enumerate()
                            .find(|(_, s)| s.state == SlotState::Idle)
                    })
                {
                    drop(slots);
                    match self
                        .prepare_slot_for_lease(i, proj_id, &session_id, &turn_id)
                        .await
                    {
                        Ok(cname) => {
                            info!(
                                target: "claw_gateway_pool",
                                component = "docker_pool",
                                phase = "acquire_slot_ok",
                                slot_index = i,
                                session_id = %session_id,
                                proj_id,
                                container = %cname,
                                "slot leased with ds bind + PG materialize"
                            );
                            let slots = self.slots.lock().await;
                            return Self::lease_from_slot(&slots, i);
                        }
                        Err(e) => {
                            warn!(
                                target: "claw_gateway_pool",
                                component = "docker_pool",
                                phase = "acquire_prepare_failed",
                                slot_index = i,
                                error = %e,
                                "pool prepare slot failed"
                            );
                            let mut slots = self.slots.lock().await;
                            if let Some(s) = slots.get_mut(i) {
                                s.state = SlotState::Dead;
                            }
                            drop(slots);
                            sleep(Duration::from_millis(200)).await;
                            continue;
                        }
                    }
                }
                let total = slots.len();
                if total < self.pool_size {
                    let idx = total;
                    let name = self.container_name(idx);
                    slots.push(Slot {
                        container_name: name.clone(),
                        state: SlotState::Idle,
                        bound_proj_id: None,
                        bound_isolation: WorkerIsolationMode::Strict,
                        test_host_root: None,
                    });
                    drop(slots);
                    let mut slots = self.slots.lock().await;
                    if let Some(s) = slots.get_mut(idx) {
                        s.state = SlotState::Leased;
                    }
                    drop(slots);
                    match self
                        .prepare_slot_for_lease(idx, proj_id, &session_id, &turn_id)
                        .await
                    {
                        Ok(cname) => {
                            info!(
                                target: "claw_gateway_pool",
                                component = "docker_pool",
                                phase = "acquire_slot_ok",
                                slot_index = idx,
                                session_id = %session_id,
                                container = %cname,
                                "new pool slot run+materialize"
                            );
                            let slots = self.slots.lock().await;
                            return Self::lease_from_slot(&slots, idx);
                        }
                        Err(e) => {
                            warn!(
                                target: "claw_gateway_pool",
                                component = "docker_pool",
                                phase = "acquire_new_slot_failed",
                                slot_index = idx,
                                error = %e,
                                "pool new slot failed"
                            );
                            let _ = self.rm_container(&name).await;
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
                if self.reconcile_stale_leased_slots().await > 0 {
                    continue;
                }
                sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .map_err(|_| "acquire_slot: timeout waiting for idle worker".to_string())?
    }

    /// `task_rel_under_root` is a path relative to the session bind root (e.g.
    /// `gateway-solve-task.json`), not under other `proj_*` trees.
    #[allow(clippy::too_many_lines)]
    pub async fn exec_solve(
        &self,
        slot: &SlotLease,
        task_rel_under_root: &str,
        claw_bin: &str,
        request_id: Option<&str>,
        turn_id: &str,
        worker_llm_env: Option<std::collections::BTreeMap<String, String>>,
        on_stdout_line: Option<Arc<dyn Fn(String) + Send + Sync>>,
    ) -> Result<TaskOutcome, String> {
        let (name, isolation) = {
            let slots = self.slots.lock().await;
            let s = slots
                .get(slot.slot_index)
                .filter(|s| s.state == SlotState::Leased)
                .ok_or_else(|| "invalid or released slot".to_string())?;
            (s.container_name.clone(), s.bound_isolation)
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
        let mut argv = self.exec_solve_argv_prefix_for(isolation);
        argv.extend([
            "-e".into(),
            "CLAW_GATEWAY_WORK_ROOT=/claw_host_root".into(),
            "-e".into(),
            format!(
                "CLAW_PROJECT_CONFIG_ROOT={}",
                session_db_sync::DS_MOUNT_TARGET
            ),
            "-e".into(),
            format!("HOME={GUEST_WORK_ROOT}"),
            "-e".into(),
            format!("XDG_CONFIG_HOME={GUEST_WORK_ROOT}/.config"),
            "-e".into(),
            format!("XDG_CACHE_HOME={GUEST_WORK_ROOT}/.cache"),
            "-e".into(),
            format!("XDG_DATA_HOME={GUEST_WORK_ROOT}/.local/share"),
        ]);
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
        if let Some(ref pool_id) = self.pool_id {
            if let Some(ref db) = self.session_db {
                let exec_user =
                    exec_user_arg_for_mode(isolation, &self.worker_identity.exec_user_arg());
                match db
                    .assign_turn_pool_worker(
                        turn_id,
                        pool_id,
                        &container_log,
                        Some(exec_user.as_str()),
                    )
                    .await
                {
                    Ok(()) => info!(
                        target: "claw_gateway_pool",
                        component = "docker_pool",
                        phase = "assign_turn_pool_worker_ok",
                        turn_id = %turn_id,
                        pool_id = %pool_id,
                        worker_name = %container_log,
                        "gateway_turns pool_id/worker_name written for live routing"
                    ),
                    Err(e) => warn!(
                        target: "claw_gateway_pool",
                        component = "docker_pool",
                        phase = "assign_turn_pool_worker_failed",
                        turn_id = %turn_id,
                        pool_id = %pool_id,
                        error = %e,
                        "gateway_turns pool_id/worker_name update failed"
                    ),
                }
            }
            argv.extend(["-e".into(), format!("CLAW_POOL_ID={pool_id}")]);
        }
        argv.extend(["-e".into(), format!("CLAW_TURN_ID={turn_id}")]);
        argv.extend(["-e".into(), format!("CLAW_WORKER_NAME={container_log}")]);
        if let Some(ref db) = self.session_db {
            if let Ok(Some(session_id)) = db.get_session_id_for_turn(turn_id).await {
                argv.extend(["-e".into(), format!("CLAW_SESSION_ID={session_id}")]);
            }
        }
        if let Some(env_map) = worker_llm_env {
            for (k, v) in env_map {
                if v.is_empty() {
                    continue;
                }
                argv.extend(["-e".into(), format!("{k}={v}")]);
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
        let stdout_hook = merge_stdout_hooks(turn_id, self.live_report_hub.clone(), on_stdout_line);
        let out = runtime_exec_with_live_streams(&self.bin, &argv_refs, request_id, stdout_hook)
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
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if let Some(ref db) = self.session_db {
            if let Ok(Some((session_id, proj_id))) = db.turn_session_scope(turn_id).await {
                let user_prompt = db
                    .get_turn_user_prompt(turn_id)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if exit_code == 0 {
                    session_db_sync::readback_out(
                        &self.bin,
                        &container_log,
                        db,
                        db.pg_pool(),
                        &session_id,
                        proj_id,
                        turn_id,
                        &user_prompt,
                    )
                    .await
                    .map_err(|e| format!("readback_out failed: {e}"))?;
                    let parsed = super::result::parse_gateway_solve_exec_stdout(&stdout, exit_code);
                    let report = parsed.output_json.as_ref().and_then(|j| {
                        crate::biz_advice_report::report_body_from_solve_output(
                            &parsed.output_text,
                            Some(j),
                        )
                        .ok()
                    });
                    session_db_sync::finalize_turn_after_readback(
                        db,
                        turn_id,
                        parsed.claw_exit_code,
                        report.as_deref(),
                        parsed.output_json.as_ref(),
                    )
                    .await
                    .map_err(|e| format!("finalize_turn_after_readback failed: {e}"))?;
                }
            }
        }
        Ok(TaskOutcome {
            exit_code,
            stdout,
            stderr,
        })
    }

    pub async fn release_slot(self: &Arc<Self>, slot: SlotLease) -> Result<(), String> {
        let (was_leased, container_name, _slot_index, isolation) = {
            let mut slots = self.slots.lock().await;
            let s = slots
                .get_mut(slot.slot_index)
                .ok_or_else(|| "release: bad slot index".to_string())?;
            let was_leased = s.state == SlotState::Leased;
            let name = s.container_name.clone();
            let idx = slot.slot_index;
            let isolation = s.bound_isolation;
            if was_leased {
                s.state = SlotState::Idle;
            }
            (was_leased, name, idx, isolation)
        };
        if was_leased {
            if let Some(ref script) = self.on_release_exec {
                if !script.trim().is_empty() {
                    self.run_on_release_hook(&container_name, script).await;
                }
            }
            self.kill_worker_solve_processes(&container_name, isolation)
                .await;
            info!(
                target: "claw_gateway_pool",
                component = "docker_pool",
                phase = "release_slot_ok",
                slot_index = slot.slot_index,
                container = %container_name,
                "slot released to Idle"
            );
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
        self.exec_solve_argv_prefix_for(WorkerIsolationMode::Strict)
    }

    pub(crate) fn test_exec_solve_argv_prefix_for(
        &self,
        isolation: WorkerIsolationMode,
    ) -> Vec<String> {
        self.exec_solve_argv_prefix_for(isolation)
    }

    pub(crate) async fn test_leased_container_name(
        &self,
        lease: &crate::pool::SlotLease,
    ) -> String {
        let slots = self.slots.lock().await;
        slots[lease.slot_index].container_name.clone()
    }
}

#[cfg(test)]
mod worker_name_stem_tests {
    use super::worker_name_stem_from_pool_suffix;

    #[test]
    fn profile_worker_hostname_stems_differ() {
        let host = "ali-hz1-onl-max-ae-schedule-11";
        let strict = worker_name_stem_from_pool_suffix(&format!("{host}-strict"));
        let relaxed = worker_name_stem_from_pool_suffix(&format!("{host}-relaxed"));
        assert_ne!(strict, relaxed);
        assert!(strict.ends_with("-strict"));
        assert!(relaxed.ends_with("-relaxed"));
        assert_eq!(
            strict,
            worker_name_stem_from_pool_suffix("ali-hz1-onl-max-ae-schedule-11-strict")
        );
    }

    #[test]
    fn legacy_pool_id_without_profile_trims_trailing_dash() {
        let stem = worker_name_stem_from_pool_suffix("ali-hz1-onl-max-ae-schedule-11");
        assert_eq!(stem, "ali-hz1-onl-max");
        assert!(!stem.ends_with('-'));
    }
}

#[cfg(test)]
mod exec_solve_argv_prefix_tests {
    use std::sync::Arc;

    use super::DockerPoolManager;
    use crate::pool::config::DockerPoolConfig;
    use crate::pool::worker_identity::PoolWorkerIdentity;
    use crate::pool::WorkerIsolationMode;

    fn pool(exec_user: Option<&str>) -> Arc<DockerPoolManager> {
        let base =
            std::env::temp_dir().join(format!("gw-exec-prefix-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&base).unwrap();
        let exec_user = exec_user.map(str::to_string);
        let worker_identity = PoolWorkerIdentity::from_env(exec_user.clone());
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
            exec_user,
            worker_identity,
            security_boost: false,
            fixed_isolation: None,
            symlink_inject: true,
            worker_env_host_file: None,
            live_report_hub: None,
            pool_id: None,
            session_db: None,
        })
        .expect("from_config")
    }

    #[test]
    fn exec_prefix_uses_uid_gid_when_unset() {
        let p = pool(None);
        let id = PoolWorkerIdentity::from_env(None);
        assert_eq!(
            p.test_exec_solve_argv_prefix(),
            vec!["exec".to_string(), "--user".to_string(), id.exec_user_arg()]
        );
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
    fn exec_prefix_relaxed_uses_pool_worker() {
        let p = pool(None);
        let id = PoolWorkerIdentity::from_env(None);
        assert_eq!(
            p.test_exec_solve_argv_prefix_for(WorkerIsolationMode::Relaxed),
            vec!["exec".to_string(), "--user".to_string(), id.exec_user_arg()]
        );
    }

    #[test]
    fn exec_prefix_falls_back_to_uid_gid_for_whitespace_user() {
        let p = pool(Some("   \t  "));
        let id = PoolWorkerIdentity::from_env(Some("   \t  ".into()));
        assert_eq!(
            p.test_exec_solve_argv_prefix(),
            vec!["exec".to_string(), "--user".to_string(), id.exec_user_arg()]
        );
    }
}

#[cfg(test)]
mod worker_volume_mount_tests {
    use std::path::Path;

    use super::DockerPoolManager;
    use crate::pool::{DS_MOUNT_TARGET, GUEST_WORK_ROOT};

    #[test]
    fn proj_home_bind_volume_arg_is_read_only() {
        let arg = DockerPoolManager::proj_home_bind_volume_arg(Path::new("/data/proj_7"));
        assert_eq!(arg, "/data/proj_7:/claw_ds:ro");
        assert!(
            !arg.contains(&format!("{DS_MOUNT_TARGET}:rw")),
            "proj_home must never be rw in worker: {arg}"
        );
    }

    #[test]
    fn guest_work_root_tmpfs_is_read_write() {
        let arg = DockerPoolManager::guest_work_root_tmpfs_arg();
        assert!(arg.starts_with(&format!("{GUEST_WORK_ROOT}:")));
        assert!(
            arg.contains(":rw,"),
            "session workspace tmpfs must be rw: {arg}"
        );
    }

    #[test]
    fn worker_bind_matches_proj_host_compares_paths() {
        let dir = std::env::temp_dir().join("claw-bind-test-proj_1");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let canon = std::fs::canonicalize(&dir).unwrap();
        assert!(DockerPoolManager::worker_bind_matches_proj_host(
            canon.to_string_lossy().as_ref(),
            &dir
        ));
        assert!(!DockerPoolManager::worker_bind_matches_proj_host(
            "/tmp/ds_1",
            &dir
        ));
        let _ = std::fs::remove_dir_all(&dir);
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
    use crate::pool::docker_cli::runtime_exec;
    use crate::pool::session_db_sync;
    use crate::pool::worker_identity::PoolWorkerIdentity;
    use crate::pool::{DS_MOUNT_TARGET, GUEST_WORK_ROOT};
    use std::sync::Arc;
    use std::time::Duration;

    fn test_pool_config(work: PathBuf, bin_path: &Path, stem: &str) -> DockerPoolConfig {
        test_pool_config_mut(work, bin_path, stem, |_| {})
    }

    fn test_pool_config_mut(
        work: PathBuf,
        bin_path: &Path,
        stem: &str,
        patch: impl FnOnce(&mut DockerPoolConfig),
    ) -> DockerPoolConfig {
        let mut cfg = DockerPoolConfig {
            runtime_bin: bin_path.to_string_lossy().into_owned(),
            work_root: work,
            pool_size: 2,
            min_idle: 0,
            image: "fake:latest".into(),
            network_args: vec![],
            extra_run_args: vec![],
            name_stem: Some(stem.into()),
            on_release_exec: None,
            exec_user: None,
            worker_identity: PoolWorkerIdentity::from_env(None),
            security_boost: false,
            fixed_isolation: None,
            symlink_inject: true,
            worker_env_host_file: None,
            live_report_hub: None,
            pool_id: None,
            session_db: None,
        };
        patch(&mut cfg);
        cfg.worker_identity = PoolWorkerIdentity::from_env(cfg.exec_user.clone());
        cfg
    }

    fn fake_docker_script(state_dir: &Path) -> String {
        let d = state_dir.to_string_lossy().replace('\'', "'\"'\"'");
        format!(
            r#"#!/bin/sh
set -eu
d='{d}'
mkdir -p "$d"
log() {{ printf '%s\n' "$*" >>"$d/log.txt"; }}
record_run_mounts() {{
  : > "$d/mounts.txt"
  prev=""
  for token in "$@"; do
    if [ "$prev" = "-v" ] || [ "$prev" = "--tmpfs" ]; then
      printf '%s\n' "$token" >>"$d/mounts.txt"
    fi
    prev="$token"
  done
}}
exec_targets_readonly_claw_ds() {{
  for arg in "$@"; do
    case "$arg" in
      /claw_ds/*|/claw_ds)
        if grep -Fq ":/claw_ds:ro" "$d/mounts.txt" 2>/dev/null; then
          echo "sh: cannot create $arg: Read-only file system" >&2
          return 2
        fi
        ;;
    esac
  done
  return 0
}}
exec_targets_readonly_project_config() {{
  for arg in "$@"; do
    case "$arg" in
      */.claw/skills/*|*/.claw/skills|*/.cursor/rules/*|*/.cursor/rules|*/CLAUDE.md)
        echo "tee: $arg: Permission denied" >&2
        return 1
        ;;
    esac
  done
  return 0
}}
case "${{1:-}}" in
run)
  log "run:$*"
  record_run_mounts "$@"
  exit 0
  ;;
exec)
  log "exec:$*"
  exec_targets_readonly_claw_ds "$@" || exit 2
  exec_targets_readonly_project_config "$@" || exit 1
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
cp)
  log "cp:$*"
  dest="${{3#*:}}"
  dest="${{dest#*/}}"
  mkdir -p "$(dirname "$d/$dest")"
  cp "$2" "$d/$dest" 2>/dev/null || cp "$2" "$dest" 2>/dev/null || true
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

    fn worker_run_line(log: &str) -> &str {
        log.lines().find(|l| l.starts_with("run:")).unwrap_or("")
    }

    fn assert_worker_mount_permissions(run_line: &str) {
        assert!(
            run_line.contains(&format!(":{DS_MOUNT_TARGET}:ro")),
            "proj_home bind must be :ro, run line:\n{run_line}"
        );
        assert!(
            !run_line.contains(&format!(":{DS_MOUNT_TARGET}:rw")),
            "proj_home must not be :rw, run line:\n{run_line}"
        );
        assert!(
            run_line.contains(&format!("{GUEST_WORK_ROOT}:rw,"))
                || run_line.contains(&format!(":{GUEST_WORK_ROOT}:rw")),
            "guest session root must stay rw, run line:\n{run_line}"
        );
    }

    use std::sync::{Mutex, MutexGuard};

    static DOCKER_POOL_IT_SERIAL: Mutex<()> = Mutex::new(());

    /// Hold for whole test: fake-docker spawns race under default lib test parallelism. Author: kejiqing
    struct DockerPoolItSerialGuard(#[allow(dead_code)] MutexGuard<'static, ()>);

    fn docker_pool_it_serial() -> DockerPoolItSerialGuard {
        DockerPoolItSerialGuard(
            DOCKER_POOL_IT_SERIAL
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    fn test_layout() -> (DockerPoolItSerialGuard, PathBuf, PathBuf, PathBuf) {
        let serial = docker_pool_it_serial();
        let base = std::env::temp_dir().join(format!("http-gw-pool-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&base).unwrap();
        let work = base.join("work");
        fs::create_dir_all(&work).unwrap();
        let state = base.join("docker_state");
        fs::create_dir_all(&state).unwrap();
        let bin_path = base.join("fake-docker");
        write_executable(&bin_path, &fake_docker_script(&state));
        (serial, base, work, bin_path)
    }

    fn write_test_task(home: &Path) {
        fs::create_dir_all(home.join(".claw")).unwrap();
        fs::write(
            home.join("gateway-solve-task.json"),
            br#"{"userPrompt":"test","turnId":"turn-test"}"#,
        )
        .unwrap();
    }

    fn test_session_id() -> String {
        format!("sess-{}", Uuid::new_v4().simple())
    }

    async fn acquire_test(
        pool: &Arc<DockerPoolManager>,
        work: &Path,
    ) -> (String, crate::pool::SlotLease) {
        let sid = test_session_id();
        let home = session_db_sync::session_home_under_work_root(work, 1, &sid);
        fs::create_dir_all(&home).unwrap();
        write_test_task(&home);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                sid.clone(),
                1,
                "turn-test".to_string(),
            )
            .await
            .unwrap();
        (sid, lease)
    }

    #[tokio::test]
    async fn acquire_exec_release_does_not_rm_worker() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "tstem"))
                .unwrap();
        let (sid, lease) = acquire_test(&pool, &work).await;
        let out = pool
            .exec_solve(
                &lease,
                "gateway-solve-task.json",
                "claw",
                None,
                "turn-test",
                None,
                None,
            )
            .await
            .unwrap();
        assert_eq!(out.exit_code, 0);
        DockerPoolManager::release_slot(&pool, lease).await.unwrap();
        let home2 = session_db_sync::session_home_under_work_root(&work, 1, &sid);
        write_test_task(&home2);
        let lease2 = pool
            .acquire_slot(
                Duration::from_secs(5),
                sid.clone(),
                1,
                "turn-test".to_string(),
            )
            .await
            .unwrap();
        DockerPoolManager::release_slot(&pool, lease2)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        let log = read_log(&state_dir);
        let slot_run_count = log
            .lines()
            .filter(|l| l.starts_with("run:") && l.contains(" infinity"))
            .count();
        assert_eq!(
            slot_run_count, 1,
            "expected single worker slot run (not chown helper), log:\n{log}"
        );
        assert!(log.contains("exec:"), "expected exec solve, log:\n{log}");
        let rm_count = log.matches("rm:").count();
        assert!(
            rm_count <= 1,
            "release must not rm worker (at most one rm before first run), got {rm_count}, log:\n{log}"
        );
    }

    #[tokio::test]
    async fn force_kill_then_ensure_warm_runs_rm_and_new_run() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "killme"))
                .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
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
        let (_serial, _base, work, bin_path) = test_layout();
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "conc"))
                .unwrap();
        let p1 = Arc::clone(&pool);
        let p2 = Arc::clone(&pool);
        let sid1 = test_session_id();
        let sid2 = test_session_id();
        let home1 = session_db_sync::session_home_under_work_root(&work, 1, &sid1);
        let home2 = session_db_sync::session_home_under_work_root(&work, 1, &sid2);
        fs::create_dir_all(&home1).unwrap();
        fs::create_dir_all(&home2).unwrap();
        write_test_task(&home1);
        write_test_task(&home2);
        let (a, b) = tokio::join!(
            p1.acquire_slot(Duration::from_secs(5), sid1, 1, "turn-test".to_string()),
            p2.acquire_slot(Duration::from_secs(5), sid2, 1, "turn-test".to_string()),
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
        let (_serial, _base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "rel",
            |c| c.pool_size = 1,
        ))
        .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        DockerPoolManager::release_slot(&pool, lease.clone())
            .await
            .unwrap();
        let err = pool
            .exec_solve(
                &lease,
                "gateway-solve-task.json",
                "claw",
                None,
                "turn-test",
                None,
                None,
            )
            .await
            .expect_err("exec on released lease must fail");
        assert!(err.contains("invalid or released"), "unexpected err: {err}");
    }

    #[tokio::test]
    async fn double_release_is_idempotent() {
        let (_serial, _base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "dbl",
            |c| c.pool_size = 1,
        ))
        .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        DockerPoolManager::release_slot(&pool, lease.clone())
            .await
            .unwrap();
        DockerPoolManager::release_slot(&pool, lease).await.unwrap();
    }

    #[tokio::test]
    async fn release_runs_configured_on_release_hook() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "relhook",
            |c| {
                c.pool_size = 1;
                c.on_release_exec = Some("echo pool_on_release".into());
            },
        ))
        .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        pool.exec_solve(
            &lease,
            "gateway-solve-task.json",
            "claw",
            None,
            "turn-test",
            None,
            None,
        )
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
    async fn release_invokes_pkill_for_solve_processes() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "pkill",
            |c| c.pool_size = 1,
        ))
        .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        pool.exec_solve(
            &lease,
            "gateway-solve-task.json",
            "claw",
            None,
            "turn-pkill",
            None,
            None,
        )
        .await
        .unwrap();
        DockerPoolManager::release_slot(&pool, lease).await.unwrap();
        let log = read_log(&state_dir);
        assert!(
            log.contains("pkill -u") && log.contains("gateway-solve-once"),
            "release must pkill solve processes, log:\n{log}"
        );
    }

    #[tokio::test]
    async fn release_pkill_uses_named_exec_user_when_configured() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "pkilluser",
            |c| {
                c.pool_size = 1;
                c.exec_user = Some("clawWorker".into());
            },
        ))
        .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        DockerPoolManager::release_slot(&pool, lease).await.unwrap();
        let log = read_log(&state_dir);
        assert!(
            log.contains("pkill -u clawWorker"),
            "pkill must follow PoolWorkerIdentity, log:\n{log}"
        );
    }

    #[tokio::test]
    async fn exec_solve_uses_uid_gid_and_claw_host_home_by_default() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "homeenv",
            |c| c.pool_size = 1,
        ))
        .unwrap();
        let id = PoolWorkerIdentity::from_env(None);
        let (_sid, lease) = acquire_test(&pool, &work).await;
        pool.exec_solve(
            &lease,
            "gateway-solve-task.json",
            "claw",
            None,
            "turn-home",
            None,
            None,
        )
        .await
        .unwrap();
        let log = read_log(&state_dir);
        assert!(
            log.contains(&format!("--user {}", id.exec_user_arg())),
            "solve exec must not run as container root, log:\n{log}"
        );
        assert!(
            log.contains("HOME=/claw_host_root")
                && log.contains("XDG_CONFIG_HOME=/claw_host_root/.config"),
            "exec env must point XDG under guest work root, log:\n{log}"
        );
    }

    #[tokio::test]
    async fn proj_home_bind_mount_is_read_only() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "dsro"))
                .unwrap();
        let (_sid, _lease) = acquire_test(&pool, &work).await;
        let log = read_log(&state_dir);
        assert_worker_mount_permissions(worker_run_line(&log));
    }

    #[tokio::test]
    async fn proj_home_bind_stays_read_only_with_security_boost() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "dsroboost",
            |c| {
                c.pool_size = 1;
                c.security_boost = true;
            },
        ))
        .unwrap();
        let (_sid, _lease) = acquire_test(&pool, &work).await;
        let log = read_log(&state_dir);
        assert_worker_mount_permissions(worker_run_line(&log));
        assert!(
            worker_run_line(&log).contains("--read-only"),
            "security boost expected with ds ro mount, log:\n{log}"
        );
    }

    #[tokio::test]
    async fn worker_cannot_write_under_claw_ds() {
        let (_serial, _base, work, bin_path) = test_layout();
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "dswrite"))
                .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        let cname = pool.test_leased_container_name(&lease).await;
        let argv = [
            "exec",
            "--user",
            "1000:1000",
            cname.as_str(),
            "tee",
            "/claw_ds/forbidden.txt",
        ];
        let bin = bin_path.to_string_lossy();
        let out = runtime_exec(bin.as_ref(), &argv).await.unwrap();
        assert!(
            !out.status.success(),
            "tee under /claw_ds must fail on ro bind, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("Read-only file system"),
            "expected EROFS-like message, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn worker_cannot_write_under_claw_proj_home() {
        let (_serial, _base, work, bin_path) = test_layout();
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "dshomewr"))
                .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        let cname = pool.test_leased_container_name(&lease).await;
        let argv = [
            "exec",
            "--user",
            "1000:1000",
            cname.as_str(),
            "tee",
            "/claw_ds/home/forbidden.txt",
        ];
        let bin = bin_path.to_string_lossy();
        let out = runtime_exec(bin.as_ref(), &argv).await.unwrap();
        assert!(
            !out.status.success(),
            "tee under /claw_ds/home must fail on ro bind, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn worker_cannot_write_project_skills() {
        let (_serial, _base, work, bin_path) = test_layout();
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "skillwr"))
                .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        let cname = pool.test_leased_container_name(&lease).await;
        let dest = format!("{DS_MOUNT_TARGET}/.claw/skills/existing/SKILL.md");
        let argv = [
            "exec",
            "--user",
            "1000:1000",
            cname.as_str(),
            "tee",
            dest.as_str(),
        ];
        let bin = bin_path.to_string_lossy();
        let out = runtime_exec(bin.as_ref(), &argv).await.unwrap();
        assert!(
            !out.status.success(),
            "tee under project .claw/skills must fail (Admin-managed), stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn worker_can_write_session_file_at_work_root() {
        let (_serial, base, work, bin_path) = test_layout();
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "gwrite"))
                .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        let cname = pool.test_leased_container_name(&lease).await;
        let dest = format!("{GUEST_WORK_ROOT}/allowed.txt");
        let argv = [
            "exec",
            "--user",
            "1000:1000",
            cname.as_str(),
            "tee",
            dest.as_str(),
        ];
        let bin = bin_path.to_string_lossy();
        let out = runtime_exec(bin.as_ref(), &argv).await.unwrap();
        assert!(
            out.status.success(),
            "tee under guest work root must succeed, stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = base;
    }

    #[tokio::test]
    async fn security_boost_appends_hardening_run_flags() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "boost",
            |c| {
                c.pool_size = 1;
                c.security_boost = true;
            },
        ))
        .unwrap();
        let (_sid, _lease) = acquire_test(&pool, &work).await;
        let log = read_log(&state_dir);
        assert!(
            log.contains("no-new-privileges")
                && log.contains("--cap-drop")
                && log.contains("ALL")
                && log.contains("--read-only")
                && log.contains("/tmp:rw,noexec,nosuid"),
            "security boost must harden worker run, log:\n{log}"
        );
    }

    #[tokio::test]
    async fn exec_solve_includes_user_when_configured() {
        let (_serial, base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "uidtest",
            |c| {
                c.pool_size = 1;
                c.exec_user = Some("claw".into());
            },
        ))
        .unwrap();
        let (_sid, lease) = acquire_test(&pool, &work).await;
        pool.exec_solve(
            &lease,
            "gateway-solve-task.json",
            "claw",
            None,
            "turn-test",
            None,
            None,
        )
        .await
        .unwrap();
        let log = read_log(&state_dir);
        assert!(
            log.contains("--user") && log.contains("claw"),
            "solve exec should pass --user claw, log:\n{log}"
        );
    }
}
