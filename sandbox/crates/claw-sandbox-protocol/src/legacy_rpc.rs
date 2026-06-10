//! Legacy line-delimited / JSON pool RPC (migration compat). Author: kejiqing

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::{SlotLease, TaskOutcome};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PoolRpcReq {
    Acquire {
        timeout_ms: u64,
        session_id: String,
        #[serde(alias = "ds_id")]
        proj_id: i64,
        turn_id: String,
    },
    Exec {
        slot_index: usize,
        task_rel: String,
        claw_bin: String,
        request_id: Option<String>,
        turn_id: String,
        #[serde(default)]
        worker_llm_env: Option<BTreeMap<String, String>>,
    },
    Release {
        slot_index: usize,
    },
    ForceKill {
        slot_index: usize,
    },
    ReportState {
        turn_id: String,
    },
    SyncTurnProgress {
        turn_id: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PoolRpcResp {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<SlotLease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TaskOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_report: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_report_at_ms: Option<i64>,
}
