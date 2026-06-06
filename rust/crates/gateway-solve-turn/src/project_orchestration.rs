//! Per-project solve orchestration (`project_config.solve_orchestration_json` → `home/.claw/solve-orchestration.json`).
//! MCP concurrency: `CLAW_MCP_MAX_CONCURRENT` only (`runtime::default_mcp_max_concurrent`). Author: kejiqing

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Relative to `ds_*` root and session worker root.
pub const SOLVE_ORCHESTRATION_CONFIG_REL: &str = "home/.claw/solve-orchestration.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SolveOrchestrationConfig {
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default = "default_planner_max_iter")]
    pub planner_max_iter: usize,
    #[serde(default = "default_writer_max_iter")]
    pub writer_max_iter: usize,
    #[serde(default)]
    pub narrator_model: Option<String>,
    #[serde(default = "default_narrator_throttle_ms")]
    pub narrator_throttle_ms: u64,
    /// Qualified MCP tool name, or raw tool name (resolved against registered tools).
    #[serde(default)]
    pub query_mcp_tool: Option<String>,
}

fn default_kind() -> String {
    String::from("single_turn")
}

fn default_planner_max_iter() -> usize {
    6
}

fn default_writer_max_iter() -> usize {
    4
}

fn default_narrator_throttle_ms() -> u64 {
    3000
}

/// Materialize orchestration JSON for worker mount (no duplicate concurrency knobs).
#[must_use]
pub fn materialize_solve_orchestration_json(value: &Value) -> Value {
    value.clone()
}

impl Default for SolveOrchestrationConfig {
    fn default() -> Self {
        Self {
            kind: default_kind(),
            planner_max_iter: default_planner_max_iter(),
            writer_max_iter: default_writer_max_iter(),
            narrator_model: None,
            narrator_throttle_ms: default_narrator_throttle_ms(),
            query_mcp_tool: None,
        }
    }
}

impl SolveOrchestrationConfig {
    #[must_use]
    pub fn is_multi_agent_analysis(&self) -> bool {
        self.kind.trim() == "multi_agent_analysis"
    }
}

/// Validate `project_config.solve_orchestration_json` before DB write.
pub fn validate_solve_orchestration_json(value: &serde_json::Value) -> Result<(), String> {
    let cfg: SolveOrchestrationConfig = serde_json::from_value(value.clone())
        .map_err(|e| format!("solveOrchestrationJson: {e}"))?;
    match cfg.kind.as_str() {
        "single_turn" | "multi_agent_analysis" => {
            if cfg.planner_max_iter == 0 || cfg.writer_max_iter == 0 {
                return Err(String::from(
                    "solveOrchestrationJson: plannerMaxIter and writerMaxIter must be >= 1",
                ));
            }
            Ok(())
        }
        other => Err(format!(
            "solveOrchestrationJson.kind must be single_turn or multi_agent_analysis, got {other:?}"
        )),
    }
}

fn parse_orchestration_file(path: &Path) -> Option<SolveOrchestrationConfig> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Resolve orchestration config for a worker session (pool ro mount or ds tree).
#[must_use]
pub fn resolve_solve_orchestration_config(session_home: &Path) -> SolveOrchestrationConfig {
    let mounted = session_home.join(SOLVE_ORCHESTRATION_CONFIG_REL);
    if let Some(cfg) = parse_orchestration_file(&mounted) {
        return cfg;
    }
    let config_root = runtime::gateway_project_config_root(session_home);
    parse_orchestration_file(&config_root.join(SOLVE_ORCHESTRATION_CONFIG_REL)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_multi_agent() {
        let v = serde_json::json!({"kind": "multi_agent_analysis"});
        validate_solve_orchestration_json(&v).unwrap();
    }

    #[test]
    fn validate_rejects_unknown_kind() {
        let v = serde_json::json!({"kind": "bogus"});
        assert!(validate_solve_orchestration_json(&v).is_err());
    }

    #[test]
    fn ignores_legacy_query_concurrency_field() {
        let v = serde_json::json!({"kind": "multi_agent_analysis", "queryConcurrency": 6});
        let cfg: SolveOrchestrationConfig = serde_json::from_value(v).unwrap();
        assert!(cfg.is_multi_agent_analysis());
    }
}
