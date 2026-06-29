//! Interactive session backends (FC cloud sandbox only). Author: kejiqing

mod fc_interactive;
mod fc_interactive_materialize;
mod fc_nas_api_singleton;
mod fc_worker_tap;
mod ttyd_url;

pub use fc_interactive_materialize::{
    build_fc_guest_writes_script, build_proj_bake_script, build_session_attach_script,
    build_start_ttyd_script,
};
pub use fc_nas_api_singleton::FcNasApiSingleton;
pub use fc_worker_tap::{
    build_fc_session_attach_with_tap, build_fc_worker_tap_start_script_from_db, fc_worker_llm_env,
    fc_worker_solve_route, resolve_fc_worker_solve_llm_route, FC_WORKER_TAP_PROXY_URL,
};

/// Admin `gateway_turns.pool_id` for OVS `@claw` interactive turns (distinct from solve `fc-cloud`). Author: kejiqing
pub const FC_INTERACTIVE_POOL_ID: &str = "fc-interactive";

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use fc_interactive::FcInteractiveBackend;
pub use ttyd_url::{terminal_ws_connect_url, TtydConnectTarget};

/// FC is the only supported interactive backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractiveBackendKind {
    Fc,
}

/// Input for starting an interactive REPL worker.
#[derive(Debug, Clone)]
pub struct InteractiveSessionSpec {
    pub session_id: String,
    /// Directory segment under `proj_N/sessions/` (same as DB `session_home` tail).
    pub session_segment: String,
    pub proj_id: i64,
    pub session_home: PathBuf,
    pub proj_home: PathBuf,
    pub llm_env: std::collections::BTreeMap<String, String>,
    pub ovs_mode: bool,
    pub start_ttyd_script: String,
    /// FC: session attach (LLM env on `/claw_host_root`); project config on `/claw_ds`.
    pub fc_session_attach_script: Option<String>,
    /// FC cold fallback: project bake when proj worker unavailable.
    pub fc_proj_bake_script: Option<String>,
}

/// True when `CLAW_INTERACTIVE_BACKEND=fc` (required; podman pool removed).
#[must_use]
pub fn interactive_backend_is_fc() -> bool {
    match std::env::var("CLAW_INTERACTIVE_BACKEND")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("fc") => true,
        Some("") | None => {
            eprintln!(
                "http-gateway-rs: CLAW_INTERACTIVE_BACKEND must be fc (local podman pool removed)"
            );
            std::process::exit(1);
        }
        Some(other) => {
            eprintln!("http-gateway-rs: invalid CLAW_INTERACTIVE_BACKEND={other:?}; use fc");
            std::process::exit(1);
        }
    }
}

/// True when `CLAW_OVS_BACKEND=fc` (OVS runs as e2b singleton, not compose).
#[must_use]
pub fn ovs_backend_is_fc() -> bool {
    std::env::var("CLAW_OVS_BACKEND")
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case("fc"))
        .unwrap_or(false)
}

/// True when FC session-observe singleton should run (Admin Live on e2b).
#[must_use]
pub fn fc_observe_is_enabled() -> bool {
    interactive_backend_is_fc()
        && !matches!(
            std::env::var("CLAW_FC_OBSERVE")
                .ok()
                .map(|v| v.trim().to_ascii_lowercase())
                .as_deref(),
            Some("0") | Some("false") | Some("no") | Some("off")
        )
}

/// Active interactive worker lease (FC sandbox).
#[derive(Debug, Clone)]
pub struct InteractiveLease {
    pub backend: InteractiveBackendKind,
    pub slot_index: usize,
    pub worker_name: Option<String>,
    pub pool_id: String,
    pub fc_sandbox_id: Option<String>,
    /// Proj worker lease marker (`fc_warm_slot` legacy name); `None` = cold sandbox.
    pub fc_warm_slot: Option<usize>,
    /// Project id for [`FcProjWorkerRegistry`] release.
    pub fc_warm_proj_id: Option<i64>,
    /// Session directory segment under `proj_N/sessions` (symlink name).
    pub fc_session_segment: Option<String>,
    /// NAS worker root id (`proj_N/workers/{id}` bind target).
    pub fc_worker_id: Option<String>,
    pub ttyd: TtydConnectTarget,
}

#[async_trait]
pub trait InteractiveSandboxBackend: Send + Sync {
    async fn start_session(&self, spec: InteractiveSessionSpec)
        -> Result<InteractiveLease, String>;
    async fn stop_session(&self, lease: &InteractiveLease) -> Result<(), String>;
}

/// Construct FC interactive backend (prefer [`super::clients::PoolClients::fc_interactive`]).
#[must_use]
pub fn interactive_backend_from_env(
    pool_clients: super::clients::PoolClients,
    _fc_client: Option<Arc<claw_fc_sandbox_client::FcSandboxClient>>,
    _pool_id: String,
    _nas_layout: crate::pool::NasLayoutBackend,
) -> Arc<dyn InteractiveSandboxBackend> {
    pool_clients.fc_interactive_arc()
}
