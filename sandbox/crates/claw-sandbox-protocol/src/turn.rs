//! Target REST turn submission (P2+). Author: kejiqing

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::isolation::IsolationMode;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCallback {
    #[serde(rename = "stdoutUrl")]
    pub stdout_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnMaterialize {
    FromPg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSpec {
    #[serde(rename = "turnId")]
    pub turn_id: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "projId")]
    pub proj_id: i64,
    pub isolation: IsolationMode,
    #[serde(rename = "clawBin", default = "default_claw_bin")]
    pub claw_bin: String,
    #[serde(rename = "workerLlmEnv", default)]
    pub worker_llm_env: BTreeMap<String, String>,
    pub callback: TurnCallback,
    pub materialize: TurnMaterialize,
}

fn default_claw_bin() -> String {
    "/usr/local/bin/claw".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTurnResponse {
    #[serde(rename = "turnId")]
    pub turn_id: String,
    pub status: String,
    #[serde(rename = "workerName", skip_serializing_if = "Option::is_none")]
    pub worker_name: Option<String>,
}
