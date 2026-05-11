//! Container pool for isolated solve (Docker / Podman). Author: kejiqing
pub mod config;
mod docker_cli;
mod docker_pool;
#[allow(dead_code)]
mod result;
pub mod rpc;
mod traits;

pub use docker_pool::DockerPoolManager;
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
