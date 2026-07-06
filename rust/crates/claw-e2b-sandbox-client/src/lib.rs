//! e2b cloud sandbox client (E2B-compatible REST + Python envd exec helper). Author: kejiqing

mod client;
mod config;
mod e2b_platform;
mod nas_paths;
mod types;

pub use client::{
    E2bSandboxClient, SandboxSnapshot, SANDBOX_LEASE_RENEW_LEAD_SECS, SANDBOX_LEASE_TICK_SECS,
    SINGLETON_ROLE_NAS_API, SINGLETON_ROLE_OBSERVE, SINGLETON_ROLE_OVS, WARM_PROJ_ROLE,
};
pub use config::E2bSandboxConfig;
pub use e2b_platform::{nas_mount_source_addr, E2bNasPlatform};
pub use nas_paths::{
    export_root_rel, guest_session_root, guest_session_work_dir, guest_worker_work_dir,
    proj_home_rel, session_ds_symlink_target, session_rel, session_symlink_target,
    sessions_root_rel, tap_traces_rel, warm_worker_mounts, worker_mounts, worker_rel,
    workers_root_rel, ovs_folder_url, ovs_workspace_folder, NasMountPoint, GUEST_CLAW_DS,
    GUEST_CLAW_HOST_ROOT, GUEST_CLAW_SESSIONS, GUEST_CLAW_TAP_TRACES, GUEST_CLAW_WS,
};
pub use types::{E2bExecOutcome, E2bSandboxHandle, GatewaySolveInputs};
