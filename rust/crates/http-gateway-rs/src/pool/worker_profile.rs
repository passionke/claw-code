//! Per-project worker profile on e2b sandbox (strict vs relaxed). Author: kejiqing

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkerProfileMode {
    #[default]
    Strict,
    Relaxed,
}

#[must_use]
pub fn default_worker_profile_json() -> Value {
    json!({"mode": "strict"})
}

/// Parse `project_config.worker_profile_json.mode`.
#[must_use]
pub fn mode_from_json(value: &Value) -> WorkerProfileMode {
    match value
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("relaxed") => WorkerProfileMode::Relaxed,
        _ => WorkerProfileMode::Strict,
    }
}

/// API label for `workerProfileJson.mode`.
#[must_use]
pub fn profile_mode_label(json: &Value) -> &'static str {
    match mode_from_json(json) {
        WorkerProfileMode::Relaxed => "relaxed",
        WorkerProfileMode::Strict => "strict",
    }
}

/// Global `CLAW_ALLOW_RELAXED_WORKER` gate + per-ds JSON.
#[must_use]
pub fn effective_mode(relaxed_allowed: bool, worker_profile_json: &Value) -> WorkerProfileMode {
    if !relaxed_allowed {
        return WorkerProfileMode::Strict;
    }
    mode_from_json(worker_profile_json)
}

/// Validate Admin `worker_profile_json` shape.
pub fn validate_worker_profile_json(value: &Value) -> Result<(), String> {
    let mode = value
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "worker_profile_json.mode required".to_string())?;
    match mode.to_ascii_lowercase().as_str() {
        "strict" | "relaxed" => Ok(()),
        other => Err(format!("invalid worker_profile_json.mode={other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_json_is_strict() {
        assert_eq!(
            mode_from_json(&default_worker_profile_json()),
            WorkerProfileMode::Strict
        );
    }

    #[test]
    fn relaxed_requires_allow_gate() {
        let json = json!({"mode": "relaxed"});
        assert_eq!(effective_mode(false, &json), WorkerProfileMode::Strict);
        assert_eq!(effective_mode(true, &json), WorkerProfileMode::Relaxed);
    }
}
