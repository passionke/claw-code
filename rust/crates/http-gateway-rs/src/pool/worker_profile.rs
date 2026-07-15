//! Per-project worker profile on e2b sandbox (strict vs relaxed). Author: kejiqing

use gateway_solve_turn::{
    default_landlock_dsl, landlock_from_worker_profile_strict, validate_landlock_dsl, LandlockDsl,
};
use serde_json::{json, Value};

use super::config::relaxed_worker_allowed_from_env;
use crate::gateway_e2b_worker_settings::validate_strict_worker_pool_size;

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

/// Optional per-project override of strict pool size (`worker_profile_json.poolSize`).
/// Absent / null → inherit global `e2bWorker.poolSize`. Author: kejiqing
#[must_use]
pub fn pool_size_override_from_json(value: &Value) -> Option<u32> {
    match value.get("poolSize") {
        None | Some(Value::Null) => None,
        Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()),
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

/// Validate project `strict.landlock` when present (system default used when omitted).
fn validate_worker_profile_strict_block(strict: &Value) -> Result<(), String> {
    if strict
        .get("useSystemDefault")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Ok(());
    }
    if let Some(dsl) = landlock_from_worker_profile_strict(strict) {
        validate_landlock_dsl(&dsl)?;
    }
    Ok(())
}

fn validate_worker_profile_pool_size(value: &Value) -> Result<(), String> {
    match value.get("poolSize") {
        None | Some(Value::Null) => Ok(()),
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| {
                "worker_profile_json.poolSize must be a positive integer".to_string()
            })?;
            let n = u32::try_from(n)
                .map_err(|_| "worker_profile_json.poolSize out of u32 range".to_string())?;
            validate_strict_worker_pool_size(n)?;
            Ok(())
        }
    }
}

/// Validate Admin `worker_profile_json` shape (+ env gate for relaxed / poolSize cap).
pub fn validate_worker_profile_json(value: &Value) -> Result<(), String> {
    let mode = value
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "worker_profile_json.mode required".to_string())?;
    match mode.to_ascii_lowercase().as_str() {
        "strict" => {
            if let Some(strict) = value.get("strict") {
                validate_worker_profile_strict_block(strict)?;
            }
            validate_worker_profile_pool_size(value)?;
            Ok(())
        }
        "relaxed" => {
            if !relaxed_worker_allowed_from_env() {
                return Err(
                    "worker_profile_json.mode=relaxed rejected: CLAW_ALLOW_RELAXED_WORKER=false \
                     (strict-only gateway); set CLAW_ALLOW_RELAXED_WORKER=true and restart, or use mode=strict"
                        .into(),
                );
            }
            if value.get("strict").is_some() {
                return Err("worker_profile_json.strict is only valid when mode=strict".into());
            }
            if !matches!(value.get("poolSize"), None | Some(Value::Null)) {
                return Err(
                    "worker_profile_json.poolSize is only valid when mode=strict (relaxed is fixed at 1)"
                        .into(),
                );
            }
            Ok(())
        }
        other => Err(format!("invalid worker_profile_json.mode={other:?}")),
    }
}

/// Validate system `strictLandlockDefault` before global settings write.
pub fn validate_system_landlock_default(dsl: &LandlockDsl) -> Result<(), String> {
    validate_landlock_dsl(dsl)
}

/// Seed for PG migration when `strictLandlockDefault` is absent.
#[must_use]
pub fn system_landlock_default_json() -> Value {
    serde_json::to_value(default_landlock_dsl()).unwrap_or_else(|_| json!({}))
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

    #[test]
    fn rejects_relaxed_with_strict_block() {
        let json = json!({"mode": "relaxed", "strict": {"useSystemDefault": true}});
        assert!(validate_worker_profile_json(&json).is_err());
    }

    #[test]
    fn accepts_strict_inherit_system() {
        let json = json!({"mode": "strict", "strict": {"useSystemDefault": true}});
        validate_worker_profile_json(&json).unwrap();
    }

    #[test]
    fn rejects_invalid_project_landlock() {
        let json = json!({
            "mode": "strict",
            "strict": {
                "landlock": {
                    "enabled": true,
                    "rw": ["/tmp"],
                    "ro": []
                }
            }
        });
        assert!(validate_worker_profile_json(&json).is_err());
    }

    #[test]
    fn accepts_strict_with_pool_size() {
        let json = json!({"mode": "strict", "poolSize": 1});
        validate_worker_profile_json(&json).unwrap();
        assert_eq!(pool_size_override_from_json(&json), Some(1));
        assert_eq!(
            pool_size_override_from_json(&json!({"mode": "strict"})),
            None
        );
    }

    #[test]
    fn rejects_relaxed_with_pool_size() {
        let json = json!({"mode": "relaxed", "poolSize": 2});
        assert!(validate_worker_profile_json(&json).is_err());
    }
}
