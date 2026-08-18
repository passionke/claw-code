//! Project-level worker env (`project_config.worker_env_json`). Author: kejiqing
//!
//! Injected only at e2b warm-proj sandbox create (`envVars`). Saving does not rotate workers.

use std::collections::BTreeMap;

use serde_json::{json, Value};

/// Default empty map.
#[must_use]
pub fn default_worker_env_json() -> Value {
    json!({})
}

pub const KB_SYNC_MIND_API_URL: &str =
    "https://mind.maxiot-inc.com/api/mind/tenants/455785b1-1358-45ce-89c0-5a66e56d7826/mcp";
pub const KB_SYNC_MIND_API_URL_KEY: &str = "CLAW_MIND_API_URL";
pub const KB_SYNC_MIND_API_AUTH_KEY: &str = "CLAW_MIND_API_AUTHORIZATION";
pub const KB_SYNC_MIND_API_HEADERS_KEY: &str = "CLAW_MIND_API_HEADERS_JSON";
pub const KB_SYNC_MIND_API_AUTHORIZATION: &str =
    "Bearer ut_Xt6k1WUor-_6EtTdmO9e9H3DlgKqWqqgLeoCn2dBHO0";

#[must_use]
pub fn default_kb_sync_worker_env_json() -> Value {
    json!({
        KB_SYNC_MIND_API_URL_KEY: KB_SYNC_MIND_API_URL,
        KB_SYNC_MIND_API_AUTH_KEY: KB_SYNC_MIND_API_AUTHORIZATION,
    })
}

#[must_use]
pub fn merge_kb_sync_worker_env_defaults(existing: &Value) -> Value {
    let mut obj = existing.as_object().cloned().unwrap_or_default();
    obj.entry(KB_SYNC_MIND_API_URL_KEY.to_string())
        .or_insert_with(|| Value::String(KB_SYNC_MIND_API_URL.to_string()));
    obj.entry(KB_SYNC_MIND_API_AUTH_KEY.to_string())
        .or_insert_with(|| Value::String(KB_SYNC_MIND_API_AUTHORIZATION.to_string()));
    Value::Object(obj)
}

/// Exact keys projects must not set (system / LLM / OTEL inject paths). Author: kejiqing
const RESERVED_EXACT: &[&str] = &["TRACEPARENT", "TRACESTATE"];

/// Key prefixes reserved for gateway / worker runtime. Author: kejiqing
const RESERVED_PREFIXES: &[&str] = &[
    "OPENAI_",
    "ANTHROPIC_",
    "CLAW_",
    "INTERNAL_CLAUDE_",
    "XAI_",
    "OPENROUTER_",
    "LANGFUSE_",
];

fn is_reserved_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    if [
        KB_SYNC_MIND_API_URL_KEY,
        KB_SYNC_MIND_API_AUTH_KEY,
        KB_SYNC_MIND_API_HEADERS_KEY,
    ]
    .iter()
    .any(|k| *k == upper)
    {
        return false;
    }
    if RESERVED_EXACT.iter().any(|k| *k == upper) {
        return true;
    }
    RESERVED_PREFIXES.iter().any(|p| upper.starts_with(p))
}

fn key_is_valid_env_name(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    // POSIX-ish: [A-Za-z_][A-Za-z0-9_]* — reject `=` / whitespace / control for shell safety.
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Validate `worker_env_json` object of string→string. Author: kejiqing
pub fn validate_worker_env_json(value: &Value) -> Result<(), String> {
    parse_worker_env_map(value).map(|_| ())
}

/// Parse into sorted map for create-time `envVars`. Author: kejiqing
pub fn parse_worker_env_map(value: &Value) -> Result<BTreeMap<String, String>, String> {
    let obj = value.as_object().ok_or_else(|| {
        "workerEnvJson must be a JSON object of string keys to string values".to_string()
    })?;
    let mut out = BTreeMap::new();
    for (key, val) in obj {
        let key = key.trim();
        if key.is_empty() {
            return Err("workerEnvJson keys must be non-empty".to_string());
        }
        if !key_is_valid_env_name(key) {
            return Err(format!(
                "workerEnvJson key `{key}` is invalid (use [A-Za-z_][A-Za-z0-9_]*)"
            ));
        }
        if is_reserved_key(key) {
            return Err(format!(
                "workerEnvJson key `{key}` is reserved (OPENAI_/ANTHROPIC_/CLAW_/…)"
            ));
        }
        let Some(s) = val.as_str() else {
            return Err(format!(
                "workerEnvJson[`{key}`] must be a string (got {})",
                val_type_name(val)
            ));
        };
        out.insert(key.to_string(), s.to_string());
    }
    Ok(out)
}

fn val_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_empty_object() {
        assert!(validate_worker_env_json(&json!({})).is_ok());
        assert!(parse_worker_env_map(&json!({})).unwrap().is_empty());
    }

    #[test]
    fn accepts_custom_keys() {
        let m = parse_worker_env_map(&json!({"FOO_BAR": "x", "biz_date": "20260101"})).unwrap();
        assert_eq!(m.get("FOO_BAR").map(String::as_str), Some("x"));
        assert_eq!(m.get("biz_date").map(String::as_str), Some("20260101"));
    }

    #[test]
    fn rejects_reserved_prefixes() {
        assert!(validate_worker_env_json(&json!({"CLAW_FOO": "1"})).is_err());
        assert!(validate_worker_env_json(&json!({"openai_api_key": "x"})).is_err());
        assert!(validate_worker_env_json(&json!({"TRACEPARENT": "00-..."})).is_err());
    }

    #[test]
    fn rejects_non_string_values() {
        assert!(validate_worker_env_json(&json!({"A": 1})).is_err());
        assert!(validate_worker_env_json(&json!({"A": null})).is_err());
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(validate_worker_env_json(&json!({"A=B": "1"})).is_err());
        assert!(validate_worker_env_json(&json!({"": "1"})).is_err());
        assert!(validate_worker_env_json(&json!({"1ABC": "1"})).is_err());
    }

    #[test]
    fn rejects_non_object() {
        assert!(validate_worker_env_json(&json!([])).is_err());
        assert!(validate_worker_env_json(&json!("x")).is_err());
    }
}
