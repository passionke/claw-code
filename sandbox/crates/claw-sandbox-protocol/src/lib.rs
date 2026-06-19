//! Shared contract between claw-gateway and claw-sandbox. Author: kejiqing

pub mod guest;
pub mod isolation;
pub mod legacy_rpc;
pub mod registry;
pub mod sandbox_rpc;
pub mod session;
pub mod turn;
pub mod types;

pub use guest::{
    resolve_guest_path, validate_guest_abs_path, GuestExecActor, GuestVolume, SlotExecIdentity,
    GUEST_LOCK_PROJECT_CONFIG_SH, GUEST_PREPARE_SESSION_WORKSPACE_SH, GUEST_WIPE_DS_SH,
    GUEST_WIPE_EPHEMERAL_MOUNTS_SH, GUEST_WIPE_WORK_ROOT_SH,
};
pub use isolation::{default_isolation_json, effective_isolation, mode_from_json, IsolationMode};
pub use legacy_rpc::{PoolRpcReq, PoolRpcResp};
pub use registry::{SandboxCapabilities, SandboxRegistration};
pub use sandbox_rpc::{
    InteractiveSessionBind, LeasedSlotInfo, ProfileCapacity, SandboxCapacity, SandboxExecChunk,
    SandboxRpcReq, SandboxRpcResp, SlotLeaseOwner,
};
pub use session::{
    DS_MOUNT_TARGET, GUEST_WORK_ROOT, WORKSPACE_TAR_ARTIFACT_KIND, WORKSPACE_TAR_ARTIFACT_PATH,
};
pub use turn::{SubmitTurnResponse, TurnCallback, TurnMaterialize, TurnSpec};
pub use types::{GuestFileBytes, SlotLease, TaskOutcome};
