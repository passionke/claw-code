//! Container pool for isolated solve (Docker / Podman). Author: kejiqing
mod docker_cli;
mod docker_pool;
mod traits;

pub use docker_pool::DockerPoolManager;
pub use traits::{SlotLease, TaskOutcome};
