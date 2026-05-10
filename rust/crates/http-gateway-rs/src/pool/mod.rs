//! Container pool for isolated solve (Docker / Podman). Author: kejiqing
pub mod config;
mod docker_cli;
mod docker_pool;
mod result;
mod traits;

pub use docker_pool::DockerPoolManager;
pub(crate) use result::parse_gateway_solve_exec_stdout;
pub use traits::{SlotLease, TaskOutcome};
