//! Docker/Podman worker pool: env read once at construction; internal `ensure_warm`. Author: kejiqing

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tracing::{info, warn};
use uuid::Uuid;

use gateway_solve_turn::WORKER_ENV_MOUNT_PATH;

use super::config::{security_boost_from_env, DockerPoolConfig};
use super::docker_cli::{runtime_exec, runtime_exec_with_live_streams};
use super::slot_mount::{self, SlotMountContext, SlotMountState};
use super::traits::{PoolSessionHostMounts, SlotLease, TaskOutcome};
use super::worker_identity::PoolWorkerIdentity;

pub const GUEST_WORK_ROOT: &str = "/claw_host_root";

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
    mount_state: SlotMountState,
}

/// Symlink inject only for the in-process fake-docker test shim (no host `mount(8)`).
/// Production: session `bind` → slot `guest/` → container `/claw_host_root` (same workspace).
fn use_symlink_inject(runtime_bin: &str) -> bool {
    runtime_bin.contains("fake-docker")
}

fn parent_dir(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| path.to_path_buf())
}

/// Pool of long-lived worker containers (Phase 2).
///
/// Each slot `run`s once with a fixed `guest/` → [`GUEST_WORK_ROOT`]. **Acquire** injects the
/// session + ds ro view via host [`slot_mount::apply`]; **release** runs [`slot_mount::teardown`].
/// `rm+run` only revives `Dead` slots or creates a new slot index. Author: kejiqing
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
        let name_stem = cfg.name_stem.unwrap_or_else(|| {
            let u = Uuid::new_v4().simple().to_string();
            u[..8].to_string()
        });
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

    fn exec_solve_argv_prefix(&self) -> Vec<String> {
        vec![
            "exec".to_string(),
            "--user".to_string(),
            self.worker_identity.exec_user_arg(),
        ]
    }

    fn slot_mount_ctx(&self, slot_index: usize) -> SlotMountContext {
        SlotMountContext {
            work_root_host: self.work_root_host.clone(),
            slot_index,
            worker_uid: self.worker_identity.uid,
            worker_gid: self.worker_identity.gid,
            symlink_inject: self.symlink_inject,
        }
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

    fn slot_guest_host_dir(&self, idx: usize) -> PathBuf {
        slot_mount::slot_guest_dir(&self.work_root_host, idx)
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

    async fn ensure_warm(self: &Arc<Self>) -> Result<(), String> {
        let _ = tokio::fs::create_dir_all(self.work_root_host.join(".claw-pool-slot")).await;
        let mut slots = self.slots.lock().await;
        for i in 0..slots.len() {
            if slots[i].state != SlotState::Dead {
                continue;
            }
            let old = slots[i].container_name.clone();
            drop(slots);
            let _ = self.rm_container(&old).await;
            let name = self.container_name(i);
            self.run_worker_slot_container(i, &name).await?;
            slots = self.slots.lock().await;
            slots[i] = Slot {
                container_name: name,
                state: SlotState::Idle,
                mount_state: SlotMountState::default(),
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
                mount_state: SlotMountState::default(),
            });
            drop(slots);
            self.run_worker_slot_container(idx, &name).await?;
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

    /// Gateway-rs often creates session dirs as root; workers need uid 1000 writable `.claw/`. Author: kejiqing
    async fn ensure_session_mount_owned_by_worker(&self, session_abs: &Path) -> Result<(), String> {
        super::session_mount_ownership::ensure_session_tree_owned_for_worker_with_runtime_fallback(
            &self.bin,
            session_abs,
        )
        .await
    }

    /// RPC / host pool: canonicalize `session_host_mount` under [`Self::work_root_host`], then uid-align. Author: kejiqing
    pub async fn chown_session_host_under_work_root(
        &self,
        session_host_mount: PathBuf,
    ) -> Result<(), String> {
        let session_abs = std::fs::canonicalize(&session_host_mount).map_err(|e| {
            format!(
                "canonicalize session chown path {}: {e}",
                session_host_mount.display()
            )
        })?;
        let root = &self.work_root_host;
        if !session_abs.starts_with(root) {
            return Err(format!(
                "session chown path {} escapes pool work_root {}",
                session_abs.display(),
                root.display()
            ));
        }
        self.ensure_session_mount_owned_by_worker(&session_abs)
            .await
    }

    /// Create or revive a slot container: fixed `guest/` → [`GUEST_WORK_ROOT`] only.
    async fn run_worker_slot_container(&self, slot_index: usize, name: &str) -> Result<(), String> {
        let guest = self.slot_guest_host_dir(slot_index);
        tokio::fs::create_dir_all(parent_dir(&guest))
            .await
            .map_err(|e| format!("mkdir slot guest parents: {e}"))?;
        tokio::fs::create_dir_all(&guest)
            .await
            .map_err(|e| format!("mkdir guest {}: {e}", guest.display()))?;
        let _ = crate::workspace_perm::chown_session_tree_for_worker(&guest);
        let guest_abs = std::fs::canonicalize(&guest)
            .map_err(|e| format!("canonicalize guest {}: {e}", guest.display()))?;
        // fake-docker / symlink_inject tests have no host mount(8); skip rshared prep.
        if !self.symlink_inject {
            slot_mount::prepare_guest_for_mount_propagation(&guest_abs)?;
        }
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
        self.append_security_boost_run_args(&mut args);
        args.push("--mount".into());
        args.push(slot_mount::guest_container_bind_mount_spec(
            &guest_abs,
            slot_mount::GUEST_CONTAINER_MOUNT_TARGET,
        ));
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
        let out = runtime_exec(&self.bin, &exec_argv)
            .await
            .map_err(|e| format!("spawn {}: {e}", self.bin))?;
        if !out.status.success() {
            warn!(
                target: "claw_gateway_pool",
                component = "docker_pool",
                phase = "worker_run_failed",
                container = %name,
                guest_bind = %guest_abs.display(),
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
            guest_bind = %guest_abs.display(),
            slot_index,
            image = %self.image,
            "{} run worker slot container ok",
            self.bin
        );
        Ok(())
    }

    fn verify_inject_visible_in_container(&self, container_name: &str) -> Result<(), String> {
        if self.symlink_inject {
            return Ok(());
        }
        slot_mount::verify_worker_container_sees_guest_file(
            &self.bin,
            container_name,
            "gateway-solve-task.json",
        )
    }

    fn inject_session_into_slot(
        &self,
        slot_index: usize,
        session_abs: &Path,
        host_mounts: &PoolSessionHostMounts,
        prior: Option<&SlotMountState>,
    ) -> Result<SlotMountState, String> {
        let ctx = self.slot_mount_ctx(slot_index);
        slot_mount::apply(&ctx, session_abs, host_mounts, prior)
    }

    async fn kill_worker_solve_processes(&self, container_name: &str) {
        let user = self.worker_identity.pkill_user();
        let script = format!("pkill -u {user} -f gateway-solve-once 2>/dev/null || true");
        self.run_on_release_hook(container_name, &script).await;
    }

    async fn rm_container(&self, name: &str) -> Result<(), String> {
        let _ = runtime_exec(&self.bin, &["rm", "-f", name]).await;
        Ok(())
    }

    /// `session_host_mount` must be `…/ds_{id}/sessions/{uuid}/` under [`Self::work_root_host`].
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
        if !session_abs.starts_with(&self.work_root_host) {
            return Err(format!(
                "session mount {} escapes pool work_root {}",
                session_abs.display(),
                self.work_root_host.display()
            ));
        }
        self.ensure_session_mount_owned_by_worker(&session_abs)
            .await?;
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
                    let prior_mount = slots[i].mount_state.clone();
                    slots[i].state = SlotState::Leased;
                    drop(slots);
                    match self.inject_session_into_slot(
                        i,
                        &session_abs,
                        &host_mounts,
                        Some(&prior_mount),
                    ) {
                        Ok(ms) => {
                            if let Err(e) = self.verify_inject_visible_in_container(&cname) {
                                warn!(
                                    target: "claw_gateway_pool",
                                    component = "docker_pool",
                                    phase = "inject_propagation_check_failed",
                                    slot_index = i,
                                    container = %cname,
                                    error = %e,
                                    "session bind not visible in worker container"
                                );
                                let mut slots = self.slots.lock().await;
                                if let Some(s) = slots.get_mut(i) {
                                    s.state = SlotState::Dead;
                                }
                                drop(slots);
                                sleep(Duration::from_millis(200)).await;
                                continue;
                            }
                            let mut slots = self.slots.lock().await;
                            if let Some(s) = slots.get_mut(i) {
                                s.mount_state = ms;
                            }
                            drop(slots);
                            info!(
                                target: "claw_gateway_pool",
                                component = "docker_pool",
                                phase = "acquire_slot_ok",
                                slot_index = i,
                                session_bind = %session_abs.display(),
                                container = %cname,
                                "idle slot injected (no rm+run)"
                            );
                            let slots = self.slots.lock().await;
                            return Self::lease_from_slot(&slots, i);
                        }
                        Err(e) => {
                            warn!(
                                target: "claw_gateway_pool",
                                component = "docker_pool",
                                phase = "inject_worker_failed",
                                slot_index = i,
                                error = %e,
                                "pool inject session into slot failed"
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
                        state: SlotState::Leased,
                        mount_state: SlotMountState::default(),
                    });
                    drop(slots);
                    match self.run_worker_slot_container(idx, &name).await {
                        Ok(()) => match self.inject_session_into_slot(
                            idx,
                            &session_abs,
                            &host_mounts,
                            None,
                        ) {
                            Ok(ms) => {
                                if let Err(e) = self.verify_inject_visible_in_container(&name) {
                                    warn!(
                                        target: "claw_gateway_pool",
                                        component = "docker_pool",
                                        phase = "inject_propagation_check_failed",
                                        slot_index = idx,
                                        container = %name,
                                        error = %e,
                                        "session bind not visible in worker container after run+inject"
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
                                let mut slots = self.slots.lock().await;
                                if let Some(s) = slots.get_mut(idx) {
                                    s.mount_state = ms;
                                }
                                drop(slots);
                                info!(
                                    target: "claw_gateway_pool",
                                    component = "docker_pool",
                                    phase = "acquire_slot_ok",
                                    slot_index = idx,
                                    session_bind = %session_abs.display(),
                                    container = %name,
                                    "new pool slot run+inject"
                                );
                                let slots = self.slots.lock().await;
                                return Self::lease_from_slot(&slots, idx);
                            }
                            Err(e) => {
                                warn!(
                                    target: "claw_gateway_pool",
                                    component = "docker_pool",
                                    phase = "inject_after_run_failed",
                                    slot_index = idx,
                                    error = %e,
                                    "pool inject after slot run failed"
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
                        },
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
        argv.extend([
            "-e".into(),
            "CLAW_GATEWAY_WORK_ROOT=/claw_host_root".into(),
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
                match db
                    .assign_turn_pool_worker(turn_id, pool_id, &container_log)
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
        Ok(TaskOutcome {
            exit_code,
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    pub async fn release_slot(self: &Arc<Self>, slot: SlotLease) -> Result<(), String> {
        let (was_leased, container_name, slot_index) = {
            let mut slots = self.slots.lock().await;
            let s = slots
                .get_mut(slot.slot_index)
                .ok_or_else(|| "release: bad slot index".to_string())?;
            let was_leased = s.state == SlotState::Leased;
            let name = s.container_name.clone();
            let idx = slot.slot_index;
            if was_leased {
                s.state = SlotState::Idle;
            }
            (was_leased, name, idx)
        };
        if was_leased {
            if let Some(ref script) = self.on_release_exec {
                if !script.trim().is_empty() {
                    self.run_on_release_hook(&container_name, script).await;
                }
            }
            self.kill_worker_solve_processes(&container_name).await;
            let ctx = self.slot_mount_ctx(slot_index);
            if let Err(e) = slot_mount::teardown(&ctx) {
                warn!(
                    target: "claw_gateway_pool",
                    component = "docker_pool",
                    phase = "slot_teardown_failed",
                    slot_index,
                    error = %e,
                    "pool slot teardown failed; marking Dead"
                );
                let mut slots = self.slots.lock().await;
                if let Some(s) = slots.get_mut(slot_index) {
                    s.state = SlotState::Dead;
                    s.mount_state = SlotMountState::default();
                }
            } else {
                let mut slots = self.slots.lock().await;
                if let Some(s) = slots.get_mut(slot_index) {
                    s.mount_state = SlotMountState::default();
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
    use crate::pool::worker_identity::PoolWorkerIdentity;

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
    fn exec_prefix_falls_back_to_uid_gid_for_whitespace_user() {
        let p = pool(Some("   \t  "));
        let id = PoolWorkerIdentity::from_env(Some("   \t  ".into()));
        assert_eq!(
            p.test_exec_solve_argv_prefix(),
            vec!["exec".to_string(), "--user".to_string(), id.exec_user_arg()]
        );
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
    use crate::pool::worker_identity::PoolWorkerIdentity;
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
        session_bind_named(work, &format!("sess-{}", Uuid::new_v4().simple()))
    }

    fn session_bind_named(work: &Path, seg: &str) -> PathBuf {
        let d = work.join("ds_1").join("sessions").join(seg);
        fs::create_dir_all(&d).unwrap();
        fs::canonicalize(&d).unwrap()
    }

    fn guest_dir(work: &Path, slot_index: usize) -> PathBuf {
        super::slot_mount::slot_guest_dir(work, slot_index)
    }

    #[tokio::test]
    async fn acquire_exec_release_does_not_rm_worker() {
        let (base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "tstem"))
                .unwrap();
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind.clone(),
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
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
        let bind2 = session_bind(&work);
        let lease2 = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind2,
                PoolSessionHostMounts::default(),
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
        assert!(
            !log.contains("rm:"),
            "release must not destroy the worker (no rm), log:\n{log}"
        );
    }

    #[tokio::test]
    async fn force_kill_then_ensure_warm_runs_rm_and_new_run() {
        let (base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "killme"))
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
        let pool =
            DockerPoolManager::from_config(test_pool_config(work.clone(), &bin_path, "conc"))
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
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "rel",
            |c| c.pool_size = 1,
        ))
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
        let (_base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "dbl",
            |c| c.pool_size = 1,
        ))
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
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
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
    async fn acquire_rejects_session_outside_work_root() {
        let (_base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "bound",
            |c| c.pool_size = 1,
        ))
        .unwrap();
        let outside =
            std::env::temp_dir().join(format!("pool-outside-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&outside).unwrap();
        let err = pool
            .acquire_slot(
                Duration::from_secs(2),
                outside,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap_err();
        assert!(
            err.contains("escapes pool work_root") || err.contains("canonicalize session"),
            "unexpected err: {err}"
        );
    }

    #[tokio::test]
    async fn same_session_reacquire_sees_updated_host_file() {
        let (_base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "cont",
            |c| c.pool_size = 1,
        ))
        .unwrap();
        let bind = session_bind_named(&work, "continuity-session");
        fs::write(bind.join("state.txt"), b"v1").unwrap();
        let lease1 = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind.clone(),
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        let guest = guest_dir(&work, lease1.slot_index);
        assert_eq!(fs::read_to_string(guest.join("state.txt")).unwrap(), "v1");
        DockerPoolManager::release_slot(&pool, lease1)
            .await
            .unwrap();
        assert!(!guest.join("state.txt").exists());

        fs::write(bind.join("state.txt"), b"v2").unwrap();
        let lease2 = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(guest.join("state.txt")).unwrap(),
            "v2",
            "续聊同 session 目录应看到宿主侧更新"
        );
        DockerPoolManager::release_slot(&pool, lease2)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn switch_session_does_not_leak_prior_guest_files() {
        let (_base, work, bin_path) = test_layout();
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "isol",
            |c| c.pool_size = 1,
        ))
        .unwrap();
        let bind_a = session_bind_named(&work, "session-a");
        let bind_b = session_bind_named(&work, "session-b");
        fs::write(bind_a.join("secret-a"), b"a").unwrap();
        fs::write(bind_b.join("note-b"), b"b").unwrap();

        let lease_a = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind_a,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        let slot_index = lease_a.slot_index;
        let guest = guest_dir(&work, slot_index);
        assert!(guest.join("secret-a").exists());
        DockerPoolManager::release_slot(&pool, lease_a)
            .await
            .unwrap();
        assert!(!guest.join("secret-a").exists());

        let lease_b = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind_b,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        assert_eq!(lease_b.slot_index, slot_index);
        assert!(
            !guest.join("secret-a").exists(),
            "切换会话后 guest 不得残留上一 session 文件"
        );
        assert!(guest.join("note-b").exists());
        DockerPoolManager::release_slot(&pool, lease_b)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn release_invokes_pkill_for_solve_processes() {
        let (base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "pkill",
            |c| c.pool_size = 1,
        ))
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
        let (base, work, bin_path) = test_layout();
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
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
        DockerPoolManager::release_slot(&pool, lease).await.unwrap();
        let log = read_log(&state_dir);
        assert!(
            log.contains("pkill -u clawWorker"),
            "pkill must follow PoolWorkerIdentity, log:\n{log}"
        );
    }

    #[tokio::test]
    async fn exec_solve_uses_uid_gid_and_claw_host_home_by_default() {
        let (base, work, bin_path) = test_layout();
        let state_dir = base.join("docker_state");
        let pool = DockerPoolManager::from_config(test_pool_config_mut(
            work.clone(),
            &bin_path,
            "homeenv",
            |c| c.pool_size = 1,
        ))
        .unwrap();
        let id = PoolWorkerIdentity::from_env(None);
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
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
    async fn security_boost_appends_hardening_run_flags() {
        let (base, work, bin_path) = test_layout();
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
        let bind = session_bind(&work);
        let _lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
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
        let (base, work, bin_path) = test_layout();
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
        let bind = session_bind(&work);
        let lease = pool
            .acquire_slot(
                Duration::from_secs(5),
                bind,
                PoolSessionHostMounts::default(),
            )
            .await
            .unwrap();
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
