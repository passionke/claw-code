//! e2b cloud sandbox pool (solve + interactive). Author: kejiqing
pub mod clients;
mod config;
mod docker_cli;
mod e2b_nas_layout;
mod e2b_nas_layout_backend;
mod e2b_orchestrated_pool;
mod e2b_proj_worker_registry;
pub mod interactive_backend;
mod live_report_hub;
mod live_report_sse;
mod result;
mod session_db_sync;
mod session_mount_ownership;
mod stdout_hooks;
mod traits;
mod worker_profile;

pub use traits::{PoolOps, SlotLease, TaskOutcome};

pub use clients::PoolClients;
pub use e2b_nas_layout::{
    allocate_worker_id, e2b_nas_layout_active, ensure_e2b_proj_nas_roots,
    ensure_proj_home_dir_on_nas, ensure_proj_sessions_root_on_nas, ensure_proj_workers_root_on_nas,
    ensure_session_root_on_nas, ensure_tap_traces_root_on_nas, ensure_worker_root_on_nas,
    nas_host_root, prepare_e2b_worker_bind_sources, proj_home_host_path, session_host_path,
    worker_host_path,
};
pub use e2b_nas_layout_backend::NasLayoutBackend;
pub use e2b_orchestrated_pool::{E2bOrchestratedPool, E2B_POOL_ID};
pub use e2b_proj_worker_registry::E2bProjWorkerRegistry;
pub use interactive_backend::E2B_INTERACTIVE_POOL_ID;
pub use interactive_backend::{
    apply_e2b_observe_worker_llm_env, build_proj_bake_script, build_session_attach_script,
    build_start_ttyd_script, e2b_observe_is_enabled, e2b_worker_llm_env, e2b_worker_solve_route,
    interactive_backend_from_env, interactive_backend_is_e2b, load_e2b_observe_proxy_base_url,
    ovs_backend_is_e2b, resolve_e2b_worker_solve_llm_route, terminal_ws_connect_url,
    E2bInteractiveBackend, E2bNasApiSingleton, InteractiveBackendKind, InteractiveLease,
    InteractiveSandboxBackend, InteractiveSessionSpec, TtydConnectTarget,
    E2B_WORKER_TAP_PLACEHOLDER_API_KEY,
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
pub use worker_profile::{
    default_worker_profile_json, effective_mode, mode_from_json, profile_mode_label,
    system_landlock_default_json, validate_system_landlock_default, validate_worker_profile_json,
    WorkerProfileMode,
};
