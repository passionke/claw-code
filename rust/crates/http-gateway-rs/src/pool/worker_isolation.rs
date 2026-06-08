//! Per-ds pool worker isolation (strict vs relaxed). Only pool-daemon reads mode. Author: kejiqing

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkerIsolationMode {
    #[default]
    Strict,
    Relaxed,
}

#[must_use]
pub fn default_worker_isolation_json() -> Value {
    json!({"mode": "strict"})
}

/// Parse `project_config.worker_isolation_json.mode`.
#[must_use]
pub fn mode_from_json(value: &Value) -> WorkerIsolationMode {
    match value
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("relaxed") => WorkerIsolationMode::Relaxed,
        _ => WorkerIsolationMode::Strict,
    }
}

/// Global `CLAW_ALLOW_RELAXED_WORKER` gate + per-ds JSON.
#[must_use]
pub fn effective_mode(relaxed_allowed: bool, worker_isolation_json: &Value) -> WorkerIsolationMode {
    if !relaxed_allowed {
        return WorkerIsolationMode::Strict;
    }
    mode_from_json(worker_isolation_json)
}

#[must_use]
pub fn exec_user_arg_for_mode(mode: WorkerIsolationMode, strict_exec_user: &str) -> String {
    match mode {
        WorkerIsolationMode::Relaxed => "0:0".to_string(),
        WorkerIsolationMode::Strict => strict_exec_user.to_string(),
    }
}

pub fn validate_worker_isolation_json(value: &Value) -> Result<(), String> {
    let Some(mode) = value.get("mode") else {
        return Err("workerIsolationJson.mode is required".into());
    };
    let Some(s) = mode.as_str() else {
        return Err("workerIsolationJson.mode must be a string".into());
    };
    match s.trim().to_ascii_lowercase().as_str() {
        "strict" | "relaxed" => Ok(()),
        other => Err(format!(
            "workerIsolationJson.mode must be \"strict\" or \"relaxed\", got {other:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_json_is_strict() {
        assert_eq!(
            mode_from_json(&default_worker_isolation_json()),
            WorkerIsolationMode::Strict
        );
    }

    #[test]
    fn relaxed_json_parses() {
        assert_eq!(
            mode_from_json(&json!({"mode": "relaxed"})),
            WorkerIsolationMode::Relaxed
        );
    }

    #[test]
    fn global_gate_overrides_relaxed() {
        assert_eq!(
            effective_mode(false, &json!({"mode": "relaxed"})),
            WorkerIsolationMode::Strict
        );
    }

    #[test]
    fn exec_user_relaxed_is_root() {
        assert_eq!(
            exec_user_arg_for_mode(WorkerIsolationMode::Relaxed, "1000:1000"),
            "0:0"
        );
    }
}
