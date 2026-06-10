//! Gateway-side client for claw-sandbox (RPC + shared pool helpers). Author: kejiqing

pub mod config;
pub mod docker_cli;
pub mod registry_env;
pub mod result;
pub mod rpc;
pub mod sandbox_rpc;
pub mod traits;
pub mod worker_isolation;

pub use claw_sandbox_protocol::{PoolRpcReq, PoolRpcResp, SlotLease, TaskOutcome};
pub use config::relaxed_worker_allowed_from_env;
pub use registry_env::{
    port_from_bind, resolve_advertise_host, resolve_gateway_base, resolve_pool_id,
};
pub use result::{parse_gateway_solve_exec_stdout, ParsedGatewaySolvePayload};
pub use rpc::PoolRpcClient;
pub use sandbox_rpc::SandboxRpcClient;
pub use traits::PoolOps;
pub use worker_isolation::{
    default_worker_isolation_json, effective_mode, exec_user_arg_for_mode, mode_from_json,
    validate_worker_isolation_json, WorkerIsolationMode,
};
