//! Worker isolation mode (strict vs relaxed). Author: kejiqing

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IsolationMode {
    #[default]
    Strict,
    Relaxed,
}

impl IsolationMode {
    /// Stable worker container profile suffix (`claw-worker-{stem}-{profile}-{n}`). Author: kejiqing
    #[must_use]
    pub fn profile_suffix(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Relaxed => "relaxed",
        }
    }
}

#[must_use]
pub fn default_isolation_json() -> Value {
    json!({"mode": "strict"})
}

#[must_use]
pub fn mode_from_json(value: &Value) -> IsolationMode {
    match value
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("relaxed") => IsolationMode::Relaxed,
        _ => IsolationMode::Strict,
    }
}

/// Global relaxed gate + per-ds JSON.
#[must_use]
pub fn effective_isolation(relaxed_allowed: bool, worker_isolation_json: &Value) -> IsolationMode {
    if !relaxed_allowed {
        return IsolationMode::Strict;
    }
    mode_from_json(worker_isolation_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_is_strict() {
        assert_eq!(
            mode_from_json(&default_isolation_json()),
            IsolationMode::Strict
        );
    }

    #[test]
    fn gate_overrides_relaxed() {
        assert_eq!(
            effective_isolation(false, &json!({"mode": "relaxed"})),
            IsolationMode::Strict
        );
    }
}
