//! Interactive session backends (podman pool vs FC cloud sandbox). Author: kejiqing

mod fc_interactive;
mod fc_interactive_materialize;
mod fc_ovs_singleton;
mod fc_warm_pool;
mod fc_worker_tap;
mod podman_interactive;
mod ttyd_url;

pub use fc_interactive_materialize::{
    build_fc_guest_writes_script, build_proj_bake_script, build_session_attach_script,
    self_hosted_proj_mount_sh, self_hosted_session_mount_sh,
};
pub use fc_ovs_singleton::FcOvsSingleton;
pub use fc_warm_pool::FcProjWarmPool;
pub use fc_worker_tap::{
    build_fc_session_attach_with_tap, build_fc_worker_tap_start_script_from_db, fc_worker_llm_env,
    fc_worker_solve_route, FC_WORKER_TAP_PROXY_URL,
};

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use claw_sandbox_protocol::IsolationMode;
use serde::{Deserialize, Serialize};

use super::clients::PoolClients;

pub use fc_interactive::FcInteractiveBackend;
pub use podman_interactive::PodmanInteractiveBackend;
pub use ttyd_url::{terminal_ws_connect_url, TtydConnectTarget};

/// Which backend holds an interactive lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractiveBackendKind {
    Podman,
    Fc,
}

/// Input for starting an interactive REPL worker.
#[derive(Debug, Clone)]
pub struct InteractiveSessionSpec {
    pub session_id: String,
    pub proj_id: i64,
    pub session_home: PathBuf,
    pub proj_home: PathBuf,
    pub llm_env: std::collections::BTreeMap<String, String>,
    pub ovs_mode: bool,
    pub sandbox_isolation: IsolationMode,
    pub start_ttyd_script: &'static str,
    /// FC: session attach (LLM env on `/claw_host_root`); project baked in warm pool.
    pub fc_session_attach_script: Option<String>,
    /// FC cold fallback: project bake when warm pool unavailable.
    pub fc_proj_bake_script: Option<String>,
}

/// True when `CLAW_INTERACTIVE_BACKEND=fc` (gateway skips host project materialize).
#[must_use]
pub fn interactive_backend_is_fc() -> bool {
    std::env::var("CLAW_INTERACTIVE_BACKEND")
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case("fc"))
        .unwrap_or(false)
}

/// True when `CLAW_OVS_BACKEND=fc` (OVS runs as e2b singleton, not compose).
#[must_use]
pub fn ovs_backend_is_fc() -> bool {
    std::env::var("CLAW_OVS_BACKEND")
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case("fc"))
        .unwrap_or(false)
}

/// Active interactive worker lease (podman slot or FC sandbox).
#[derive(Debug, Clone)]
pub struct InteractiveLease {
    pub backend: InteractiveBackendKind,
    pub slot_index: usize,
    pub worker_name: Option<String>,
    pub pool_id: String,
    pub fc_sandbox_id: Option<String>,
    /// Warm-pool slot index when leased from [`FcProjWarmPool`]; `None` = cold sandbox.
    pub fc_warm_slot: Option<usize>,
    /// Project id for warm-pool release / top-up.
    pub fc_warm_proj_id: Option<i64>,
    pub ttyd: TtydConnectTarget,
}

#[async_trait]
pub trait InteractiveSandboxBackend: Send + Sync {
    async fn start_session(&self, spec: InteractiveSessionSpec)
        -> Result<InteractiveLease, String>;
    async fn stop_session(&self, lease: &InteractiveLease) -> Result<(), String>;
}

/// Construct backend from `CLAW_INTERACTIVE_BACKEND` (`podman` default, `fc` for cloud).
#[must_use]
pub fn interactive_backend_from_env(
    pool_clients: PoolClients,
    fc_client: Option<Arc<claw_fc_sandbox_client::FcSandboxClient>>,
    pool_id: String,
    pool_rpc_host_work_root: Option<PathBuf>,
    work_root: PathBuf,
) -> Arc<dyn InteractiveSandboxBackend> {
    let mode = std::env::var("CLAW_INTERACTIVE_BACKEND")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_else(|| "podman".into());
    match mode.as_str() {
        "fc" => {
            let client = fc_client.unwrap_or_else(|| {
                eprintln!(
                    "http-gateway-rs: CLAW_INTERACTIVE_BACKEND=fc but FC client not configured"
                );
                std::process::exit(1);
            });
            Arc::new(FcInteractiveBackend::new(client, pool_id))
                as Arc<dyn InteractiveSandboxBackend>
        }
        "podman" | "" => {
            let sandbox = pool_clients.sandbox_rpc_client().unwrap_or_else(|| {
                eprintln!(
                    "http-gateway-rs: CLAW_INTERACTIVE_BACKEND=podman requires claw-sandbox RPC"
                );
                std::process::exit(1);
            });
            Arc::new(PodmanInteractiveBackend::new(
                sandbox,
                pool_id,
                work_root,
                pool_rpc_host_work_root,
            )) as Arc<dyn InteractiveSandboxBackend>
        }
        other => {
            eprintln!(
                "http-gateway-rs: invalid CLAW_INTERACTIVE_BACKEND={other:?}; use podman or fc"
            );
            std::process::exit(1);
        }
    }
}
