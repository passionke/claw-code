//! FC cloud sandbox pool (solve + interactive). Author: kejiqing
pub mod clients;
mod config;
mod docker_cli;
mod fc_nas_layout;
mod fc_nas_layout_backend;
mod fc_orchestrated_pool;
mod fc_proj_worker_registry;
pub mod interactive_backend;
mod live_report_hub;
mod live_report_sse;
mod result;
mod session_db_sync;
mod session_mount_ownership;
mod stdout_hooks;
mod traits;
mod worker_isolation;

pub use traits::{PoolOps, SlotLease, TaskOutcome};

pub use clients::PoolClients;
pub use fc_nas_layout::{
    allocate_worker_id, ensure_fc_proj_nas_roots, ensure_proj_home_dir_on_nas,
    ensure_proj_sessions_root_on_nas, ensure_proj_workers_root_on_nas, ensure_session_root_on_nas,
    ensure_tap_traces_root_on_nas, ensure_worker_root_on_nas, fc_nas_layout_active, nas_host_root,
    prepare_fc_worker_bind_sources, proj_home_host_path, session_host_path, worker_host_path,
};
pub use fc_nas_layout_backend::NasLayoutBackend;
pub use fc_orchestrated_pool::{FcOrchestratedPool, FC_POOL_ID};
pub use fc_proj_worker_registry::FcProjWorkerRegistry;
pub use interactive_backend::FC_INTERACTIVE_POOL_ID;
pub use interactive_backend::{
    build_fc_session_attach_with_tap, build_proj_bake_script, build_session_attach_script,
    build_start_ttyd_script, fc_observe_is_enabled, fc_worker_llm_env, fc_worker_solve_route,
    interactive_backend_from_env, interactive_backend_is_fc, ovs_backend_is_fc,
    resolve_fc_worker_solve_llm_route, terminal_ws_connect_url, FcInteractiveBackend,
    FcNasApiSingleton, InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend,
    InteractiveSessionSpec, TtydConnectTarget, FC_WORKER_TAP_PROXY_URL,
};
pub use live_report_hub::{HubMsg, LiveReportHub};
pub use live_report_sse::live_report_sse_response;
#[allow(unused_imports)]
pub use result::parse_gateway_solve_exec_stdout;
pub use session_db_sync::{
    bootstrap_empty_solve_session_jsonl, gateway_proj_work_dir, gateway_session_home,
    nas_cluster_id, proj_work_dir, session_home_under_work_root, DS_MOUNT_TARGET,
    GUEST_CLAW_SESSIONS, WORKSPACE_TAR_ARTIFACT_KIND, WORKSPACE_TAR_ARTIFACT_PATH,
};
pub use session_mount_ownership::{
    ensure_session_tree_owned_for_worker_with_runtime_fallback, path_for_pool_acquire,
};
pub use stdout_hooks::merge_stdout_hooks;
pub use worker_isolation::{
    default_worker_isolation_json, execution_backend_from_json, is_fc_sandbox_mode,
    isolation_mode_label, validate_worker_isolation_json, WorkerExecutionBackend,
    WorkerIsolationMode,
};
