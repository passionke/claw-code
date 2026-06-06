//! Container pool for isolated solve (Docker / Podman). Author: kejiqing
pub mod config;
mod docker_cli;
mod docker_pool;
mod http_server;
mod live_report_hub;
mod live_report_sse;
mod local_ops;
#[allow(dead_code)]
mod result;
pub mod rpc;
mod session_db_sync;
mod session_mount_ownership;
mod traits;
mod worker_identity;
pub use docker_pool::{merge_stdout_hooks, DockerPoolManager};
pub use http_server::serve_pool_http;
pub use live_report_hub::{HubMsg, LiveReportHub};
pub use local_ops::LocalPoolOps;
pub use session_db_sync::{
    read_worker_progress_artifacts, MaterializeInput, DS_MOUNT_TARGET, GUEST_WORK_ROOT,
    WORKSPACE_TAR_ARTIFACT_KIND, WORKSPACE_TAR_ARTIFACT_PATH,
};
pub use session_mount_ownership::ensure_session_tree_owned_for_worker_with_runtime_fallback;
pub use worker_identity::PoolWorkerIdentity;
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
