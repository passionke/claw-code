//! Worker pool implementation (Docker / Podman). Author: kejiqing

mod config;
mod docker_pool;
mod guest_io;
mod http_server;
mod live_report_hub;
mod local_ops;
mod rpc;
mod sandbox_rpc;
mod sandbox_stream;
mod worker_identity;
mod worker_isolation;

pub use claw_sandbox_protocol::GUEST_WORK_ROOT;
pub use claw_sandbox_protocol::{PoolRpcReq, PoolRpcResp, SandboxRpcReq, SandboxRpcResp};
pub use config::{
    fixed_isolation_from_env, relaxed_worker_allowed_from_env, security_boost_from_env,
    DockerPoolConfig,
};
pub use docker_pool::{merge_stdout_hooks, DockerPoolManager};
pub use guest_io::{
    extract_tar_b64_under_prefix, read_files_base64, wipe_guest_ephemeral_mounts,
    wipe_guest_work_root, write_file_via_exec_user,
};
pub use http_server::serve_pool_http;
pub use live_report_hub::{HubMsg, LiveReportHub};
pub use local_ops::LocalPoolOps;
pub use rpc::{
    dispatch_pool_rpc, handle_pool_rpc_connection, handle_pool_rpc_tcp_connection, serve_pool_rpc,
    serve_pool_rpc_tcp,
};
pub use sandbox_rpc::dispatch_sandbox_rpc;
pub use worker_identity::PoolWorkerIdentity;
