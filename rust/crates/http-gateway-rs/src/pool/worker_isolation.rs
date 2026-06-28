//! Per-project worker execution profile metadata (FC-only runtime). Author: kejiqing

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkerIsolationMode {
    #[default]
    Strict,
    Relaxed,
}

/// Solve always runs on FC cloud sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerExecutionBackend {
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

/// FC is the only execution backend.
#[must_use]
pub fn execution_backend_from_json(_value: &Value) -> WorkerExecutionBackend {
    WorkerExecutionBackend::FcSandbox
}

/// API label for `workerIsolationJson.mode`.
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

/// Global `CLAW_ALLOW_RELAXED_WORKER` gate + per-ds JSON (metadata only).
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

/// Validate Admin `worker_isolation_json` shape.
pub fn validate_worker_isolation_json(value: &Value) -> Result<(), String> {
    let mode = value
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "worker_isolation_json.mode required".to_string())?;
    match mode.to_ascii_lowercase().as_str() {
        "strict" | "relaxed" | "sandbox" => Ok(()),
        other => Err(format!("invalid worker_isolation_json.mode={other:?}")),
    }
}
