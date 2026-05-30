//! Deprecated: worker LLM env is injected per-solve via pool Exec (see claw_tap_cluster_state). Author: kejiqing

use tracing::info;

use crate::gateway_llm_config_sync::LlmRuntimeHandle;
use crate::session_db::GatewaySessionDb;

/// No-op retained for pool-daemon startup hook compatibility. Author: kejiqing
pub fn sync_pool_worker_runtime_from_db(
    _db: &GatewaySessionDb,
    _llm_handle: &LlmRuntimeHandle,
) -> Result<(), String> {
    Ok(())
}

pub fn pool_worker_runtime_poll_interval_secs() -> u64 {
    0
}

pub fn pool_worker_runtime_poll_loop(
    _db: std::sync::Arc<GatewaySessionDb>,
    _llm_handle: LlmRuntimeHandle,
) {
    info!(
        target: "claw_gateway_pool",
        component = "pool_worker_runtime_sync",
        "pool worker LLM file sync disabled; gateway injects LLM env per Exec"
    );
}

pub fn resolve_repo_root() -> Option<std::path::PathBuf> {
    if let Ok(raw) = std::env::var("CLAW_REPO_ROOT") {
        let p = std::path::PathBuf::from(raw.trim());
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    None
}
