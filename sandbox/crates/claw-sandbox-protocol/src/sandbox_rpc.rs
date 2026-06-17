//! End-state sandbox pool RPC (remote-friendly, no session semantics). Author: kejiqing

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::guest::GuestExecActor;
use crate::guest::GuestVolume;
use crate::isolation::IsolationMode;
use crate::types::{GuestFileBytes, SlotLease, TaskOutcome};

/// Exec RPC defaults to streaming stdout relay (mechanical pipe). Author: kejiqing
#[must_use]
pub fn default_stream_true() -> bool {
    true
}

/// Who holds a leased worker slot (pool in-memory truth). Author: kejiqing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SlotLeaseOwner {
    Terminal { session_id: String, proj_id: i64 },
    Solve { turn_id: String, proj_id: i64 },
}

impl SlotLeaseOwner {
    #[must_use]
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Terminal { .. } => "terminal",
            Self::Solve { .. } => "solve",
        }
    }
}

/// One leased slot reported by pool `list_leased`. Author: kejiqing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeasedSlotInfo {
    pub slot_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
    pub worker_profile: IsolationMode,
    /// `None` = orphan (legacy acquire or metadata lost).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<SlotLeaseOwner>,
}

/// Interactive `/coding` worker: bind host proj + session dirs; publish ttyd on loopback. Author: kejiqing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractiveSessionBind {
    /// Host path `work_root/proj_{id}` (ro → `/claw_ds`).
    pub proj_home_host: String,
    /// Host path to session workspace (rw → `/claw_host_root`).
    pub session_host_root: String,
    /// Host loopback port published as `127.0.0.1:{port}:7681`.
    pub ttyd_host_port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SandboxRpcReq {
    Acquire {
        isolation: IsolationMode,
        timeout_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interactive: Option<InteractiveSessionBind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owner: Option<SlotLeaseOwner>,
    },
    /// Pool root wipes both tmpfs mounts (`/claw_ds` + `/claw_host_root`).
    GuestWipe {
        slot_index: usize,
    },
    /// Write bytes under a [`GuestVolume`] + relative path (no raw `/claw_*` paths).
    GuestWrite {
        slot_index: usize,
        volume: GuestVolume,
        rel_path: String,
        bytes_b64: String,
    },
    GuestExtractTarB64 {
        slot_index: usize,
        volume: GuestVolume,
        #[serde(default)]
        rel_path_prefix: String,
        tar_b64: String,
    },
    GuestRead {
        slot_index: usize,
        guest_paths: Vec<String>,
    },
    /// Remove legacy `home/` mirror under session workspace before materialize.
    GuestPrepareSessionWorkspace {
        slot_index: usize,
    },
    /// chmod `/claw_ds` read-only for worker user after project_config writes.
    GuestLockProjectConfig {
        slot_index: usize,
    },
    /// Escape hatch (e.g. workspace tar pack); prefer typed ops above.
    GuestExecSh {
        slot_index: usize,
        script: String,
        #[serde(default)]
        actor: GuestExecActor,
    },
    Exec {
        slot_index: usize,
        argv: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default = "default_stream_true")]
        stream: bool,
    },
    ExecSolve {
        slot_index: usize,
        task_rel: String,
        claw_bin: String,
        turn_id: String,
        #[serde(default)]
        worker_llm_env: Option<BTreeMap<String, String>>,
        #[serde(default = "default_stream_true")]
        stream: bool,
    },
    Release {
        slot_index: usize,
    },
    ForceKill {
        slot_index: usize,
    },
    Capacity,
    ListLeased,
}

/// Per-profile worker counts (strict / relaxed / …). Author: kejiqing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileCapacity {
    pub profile: IsolationMode,
    pub slots_max: usize,
    pub slots_idle: usize,
    pub slots_leased: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxCapacity {
    pub slots_max: usize,
    pub slots_idle: usize,
    pub slots_leased: usize,
    /// Per-profile breakdown; one pool daemon, dedicated workers per profile. Author: kejiqing
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<ProfileCapacity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxExecChunk {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxRpcResp {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<SlotLease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TaskOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<GuestFileBytes>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<SandboxCapacity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec_chunk: Option<SandboxExecChunk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub leased_slots: Option<Vec<LeasedSlotInfo>>,
}
