//! Per-phase wall-clock timings for multi-agent solve. Author: kejiqing

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const TIMINGS_REL: &str = ".claw/multi-agent-timings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseTiming {
    pub phase: String,
    #[serde(rename = "startedAtMs")]
    pub started_at_ms: i64,
    #[serde(rename = "endedAtMs")]
    pub ended_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiAgentTimings {
    pub phases: Vec<PhaseTiming>,
}

impl MultiAgentTimings {
    pub fn load(session_home: &Path) -> Self {
        let path = session_home.join(TIMINGS_REL);
        if let Ok(raw) = fs::read_to_string(path) {
            if let Ok(t) = serde_json::from_str(&raw) {
                return t;
            }
        }
        Self { phases: Vec::new() }
    }

    pub fn push(&mut self, phase: &str, started_ms: i64, ended_ms: i64, detail: Option<String>) {
        self.phases.push(PhaseTiming {
            phase: phase.to_string(),
            started_at_ms: started_ms,
            ended_at_ms: ended_ms,
            detail,
        });
    }

    pub fn save(&self, session_home: &Path) -> Result<(), String> {
        let path = session_home.join(TIMINGS_REL);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create timings dir: {e}"))?;
        }
        let bytes =
            serde_json::to_vec_pretty(self).map_err(|e| format!("serialize timings: {e}"))?;
        fs::write(path, bytes).map_err(|e| format!("write timings: {e}"))?;
        Ok(())
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
