//! Interactive session backends (podman pool vs FC cloud sandbox). Author: kejiqing

mod fc_interactive;
mod podman_interactive;
mod ttyd_url;

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
}

/// Active interactive worker lease (podman slot or FC sandbox).
#[derive(Debug, Clone)]
pub struct InteractiveLease {
    pub backend: InteractiveBackendKind,
    pub slot_index: usize,
    pub worker_name: Option<String>,
    pub pool_id: String,
    pub fc_sandbox_id: Option<String>,
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
            let _ = pool_clients.sandbox_rpc_client().unwrap_or_else(|| {
                eprintln!(
                    "http-gateway-rs: CLAW_INTERACTIVE_BACKEND=podman requires claw-sandbox RPC"
                );
                std::process::exit(1);
            });
            Arc::new(PodmanInteractiveBackend::new(
                pool_clients,
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
