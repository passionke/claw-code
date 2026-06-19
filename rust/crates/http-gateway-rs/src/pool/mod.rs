//! Container pool for isolated solve (Docker / Podman). Author: kejiqing
pub mod clients;
mod config;
mod docker_cli;
mod docker_pool;
mod guest_materialize_tar;
mod http_server;
pub mod interactive_backend;
mod live_report_hub;
mod live_report_sse;
mod local_ops;
#[allow(dead_code)]
mod result;
pub mod rpc;
pub mod sandbox_orchestrator;
mod session_db_sync;
mod session_mount_ownership;
mod traits;
mod worker_identity;
pub mod worker_isolation;
pub use clients::PoolClients;
pub use docker_pool::{merge_stdout_hooks, DockerPoolManager};
pub use http_server::serve_pool_http;
pub use interactive_backend::{
    interactive_backend_from_env, terminal_ws_connect_url, InteractiveBackendKind,
    InteractiveLease, InteractiveSandboxBackend, InteractiveSessionSpec, TtydConnectTarget,
};
pub use live_report_hub::{HubMsg, LiveReportHub};
pub use live_report_sse::live_report_sse_response;
pub use local_ops::LocalPoolOps;
pub use sandbox_orchestrator::{worker_isolation_to_sandbox, SandboxOrchestratedPool};
pub use session_db_sync::{
    proj_work_dir, read_worker_progress_artifacts, session_home_under_work_root, MaterializeInput,
    DS_MOUNT_TARGET, GUEST_WORK_ROOT, WORKSPACE_TAR_ARTIFACT_KIND, WORKSPACE_TAR_ARTIFACT_PATH,
};
pub use session_mount_ownership::{
    ensure_session_tree_owned_for_worker_with_runtime_fallback, path_for_pool_acquire,
};
pub use worker_identity::PoolWorkerIdentity;
pub use worker_isolation::{
    default_worker_isolation_json, isolation_mode_label, validate_worker_isolation_json,
    WorkerIsolationMode,
};
// Used by the `http-gateway-rs` binary (`solve_pool`); not referenced from the library target alone.
#[allow(unused_imports)]
pub use result::parse_gateway_solve_exec_stdout;
// `serve_pool_rpc` / `handle_pool_rpc_connection` are for `claw-pool-daemon` and tests.
#[allow(unused_imports)]
pub use rpc::{
    handle_pool_rpc_connection, handle_pool_rpc_tcp_connection, serve_pool_rpc, serve_pool_rpc_tcp,
    PoolRpcClient,
};
#[allow(unused_imports)]
pub use traits::{PoolOps, SlotLease, TaskOutcome};
