//! Core pool slot / exec result types. Author: kejiqing

use serde::{Deserialize, Serialize};

use crate::guest::SlotExecIdentity;
use crate::isolation::IsolationMode;

/// Lease for one worker slot (index into the pool).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlotLease {
    pub slot_index: usize,
    /// Immutable worker container profile for this slot (strict / relaxed / …). Author: kejiqing
    pub worker_profile: IsolationMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
    /// `podman exec --user` for this lease (RPC boundary). Author: kejiqing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_identity: Option<SlotExecIdentity>,
}

/// Opaque bytes read from a guest path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestFileBytes {
    pub path: String,
    pub bytes_b64: String,
}

/// Result of `docker exec` running `claw gateway-solve-once`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
