//! Per-project worker execution profile (podman pool strict/relaxed vs FC cloud sandbox). Author: kejiqing

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkerIsolationMode {
    #[default]
    Strict,
    Relaxed,
}

/// Where the project runs workers (podman pool vs FC cloud sandbox).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerExecutionBackend {
    PodmanPool { isolation: WorkerIsolationMode },
    FcSandbox,
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
        Some("sandbox") => WorkerIsolationMode::Strict,
        _ => WorkerIsolationMode::Strict,
    }
}

/// True when Admin selected FC cloud sandbox for this project.
#[must_use]
pub fn is_fc_sandbox_mode(value: &Value) -> bool {
    value
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| s.eq_ignore_ascii_case("sandbox"))
        .is_some()
}

/// Resolve podman profile vs FC sandbox from project JSON.
#[must_use]
pub fn execution_backend_from_json(value: &Value) -> WorkerExecutionBackend {
    if is_fc_sandbox_mode(value) {
        return WorkerExecutionBackend::FcSandbox;
    }
    WorkerExecutionBackend::PodmanPool {
        isolation: mode_from_json(value),
    }
}

/// API label for `workerIsolationJson.mode` (requested ds isolation).
#[must_use]
pub fn isolation_mode_label(json: &Value) -> &'static str {
    if is_fc_sandbox_mode(json) {
        return "sandbox";
    }
    match mode_from_json(json) {
        WorkerIsolationMode::Relaxed => "relaxed",
        WorkerIsolationMode::Strict => "strict",
    }
}

/// Global `CLAW_ALLOW_RELAXED_WORKER` gate + per-ds JSON (podman pool only).
#[must_use]
pub fn effective_mode(relaxed_allowed: bool, worker_isolation_json: &Value) -> WorkerIsolationMode {
    if is_fc_sandbox_mode(worker_isolation_json) {
        return WorkerIsolationMode::Strict;
    }
    if !relaxed_allowed {
        return WorkerIsolationMode::Strict;
    }
    mode_from_json(worker_isolation_json)
}

#[must_use]
pub fn exec_user_arg_for_mode(mode: WorkerIsolationMode, pool_exec_user: &str) -> String {
    match mode {
        WorkerIsolationMode::Relaxed => "0:0".to_string(),
        WorkerIsolationMode::Strict => pool_exec_user.to_string(),
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
        "strict" | "relaxed" | "sandbox" => Ok(()),
        other => Err(format!(
            "workerIsolationJson.mode must be \"strict\", \"relaxed\", or \"sandbox\", got {other:?}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_json_is_strict_podman() {
        let json = default_worker_isolation_json();
        assert_eq!(mode_from_json(&json), WorkerIsolationMode::Strict);
        assert!(matches!(
            execution_backend_from_json(&json),
            WorkerExecutionBackend::PodmanPool {
                isolation: WorkerIsolationMode::Strict
            }
        ));
    }

    #[test]
    fn sandbox_mode_routes_fc() {
        let json = json!({"mode": "sandbox"});
        assert!(is_fc_sandbox_mode(&json));
        assert_eq!(isolation_mode_label(&json), "sandbox");
        assert_eq!(
            execution_backend_from_json(&json),
            WorkerExecutionBackend::FcSandbox
        );
    }

    #[test]
    fn relaxed_json_parses_podman() {
        assert!(matches!(
            execution_backend_from_json(&json!({"mode": "relaxed"})),
            WorkerExecutionBackend::PodmanPool {
                isolation: WorkerIsolationMode::Relaxed
            }
        ));
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
            exec_user_arg_for_mode(WorkerIsolationMode::Relaxed, "claw"),
            "0:0"
        );
    }
}
