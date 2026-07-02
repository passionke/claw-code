//! Interactive session backends (e2b sandbox only). Author: kejiqing

mod e2b_interactive;
mod e2b_interactive_materialize;
mod e2b_nas_api_singleton;
mod e2b_worker_tap;
mod ttyd_url;

pub use e2b_interactive_materialize::{
    build_e2b_guest_writes_script, build_proj_bake_script, build_session_attach_script,
    build_start_ttyd_script,
};
pub use e2b_nas_api_singleton::E2bNasApiSingleton;
pub use e2b_worker_tap::{
    apply_e2b_observe_worker_llm_env, e2b_worker_llm_env, e2b_worker_solve_route,
    load_e2b_observe_proxy_base_url, resolve_e2b_worker_solve_llm_route,
    E2B_WORKER_TAP_PLACEHOLDER_API_KEY,
};

/// Admin `gateway_turns.pool_id` for OVS `@claw` interactive turns (distinct from solve `e2b-cloud`). Author: kejiqing
pub const E2B_INTERACTIVE_POOL_ID: &str = "e2b-interactive";

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use e2b_interactive::E2bInteractiveBackend;
pub use ttyd_url::{terminal_ws_connect_url, TtydConnectTarget};

/// e2b is the only supported interactive backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractiveBackendKind {
    E2b,
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
    pub e2b_session_attach_script: Option<String>,
    /// e2b cold fallback: project bake when proj worker unavailable.
    pub e2b_proj_bake_script: Option<String>,
}

/// True when `CLAW_INTERACTIVE_BACKEND=e2b` (required; podman pool removed).
#[must_use]
pub fn interactive_backend_is_e2b() -> bool {
    match std::env::var("CLAW_INTERACTIVE_BACKEND")
        .ok()
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("e2b") => true,
        Some("") | None => {
            eprintln!(
                "http-gateway-rs: CLAW_INTERACTIVE_BACKEND must be e2b (local podman pool removed)"
            );
            std::process::exit(1);
        }
        Some(other) => {
            eprintln!("http-gateway-rs: invalid CLAW_INTERACTIVE_BACKEND={other:?}; use e2b");
            std::process::exit(1);
        }
    }
}

/// True when `CLAW_OVS_BACKEND=e2b` (OVS runs as e2b singleton, not compose).
#[must_use]
pub fn ovs_backend_is_e2b() -> bool {
    std::env::var("CLAW_OVS_BACKEND")
        .ok()
        .map(|v| v.trim().eq_ignore_ascii_case("e2b"))
        .unwrap_or(false)
}

/// True when e2b session-observe singleton should run (Admin Live on e2b).
#[must_use]
pub fn e2b_observe_is_enabled() -> bool {
    interactive_backend_is_e2b()
        && !matches!(
            std::env::var("CLAW_E2B_OBSERVE")
                .ok()
                .map(|v| v.trim().to_ascii_lowercase())
                .as_deref(),
            Some("0" | "false" | "no" | "off")
        )
}

/// Active interactive worker lease (e2b sandbox).
#[derive(Debug, Clone)]
pub struct InteractiveLease {
    pub backend: InteractiveBackendKind,
    pub slot_index: usize,
    pub worker_name: Option<String>,
    pub pool_id: String,
    pub e2b_sandbox_id: Option<String>,
    /// Proj worker lease marker (`e2b_warm_slot` legacy name); `None` = cold sandbox.
    pub e2b_warm_slot: Option<usize>,
    /// Project id for [`E2bProjWorkerRegistry`] release.
    pub e2b_warm_proj_id: Option<i64>,
    /// Session directory segment under `proj_N/sessions` (symlink name).
    pub e2b_session_segment: Option<String>,
    /// NAS worker root id (`proj_N/workers/{id}` bind target).
    pub e2b_worker_id: Option<String>,
    pub ttyd: TtydConnectTarget,
}

#[async_trait]
pub trait InteractiveSandboxBackend: Send + Sync {
    async fn start_session(&self, spec: InteractiveSessionSpec)
        -> Result<InteractiveLease, String>;
    async fn stop_session(&self, lease: &InteractiveLease) -> Result<(), String>;
}

/// Construct e2b interactive backend (prefer [`super::clients::PoolClients::e2b_interactive`]).
#[must_use]
pub fn interactive_backend_from_env(
    pool_clients: &super::clients::PoolClients,
    _e2b_client: Option<Arc<claw_e2b_sandbox_client::E2bSandboxClient>>,
    _pool_id: String,
    _nas_layout: crate::pool::NasLayoutBackend,
) -> Arc<dyn InteractiveSandboxBackend> {
    pool_clients.e2b_interactive_arc()
}
