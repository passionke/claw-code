//! Project role `scope`: Spring-style custom scope over extraSession keys. Author: kejiqing

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};

use crate::project_extra_session::{parse_extra_session_fields_json, validate_field_key};

/// Default idle seconds before pause when `idleSleepSecs` omitted.
pub const DEFAULT_IDLE_SLEEP_SECS: u64 = 900;

/// Fallback upper bound for concurrent scope workers per project.
pub const SCOPE_WORKER_CAP_DEFAULT: u32 = 64;

/// Env: `CLAW_E2B_SCOPE_WORKER_CAP` (default [`SCOPE_WORKER_CAP_DEFAULT`]).
#[must_use]
pub fn scope_worker_cap_from_env() -> u32 {
    std::env::var("CLAW_E2B_SCOPE_WORKER_CAP")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(SCOPE_WORKER_CAP_DEFAULT)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectScopeConfig {
    pub scope_keys: Vec<String>,
    pub idle_sleep_secs: u64,
}

impl ProjectScopeConfig {
    #[must_use]
    pub fn idle_sleep_ms(&self) -> i64 {
        i64::try_from(self.idle_sleep_secs.saturating_mul(1000)).unwrap_or(i64::MAX)
    }
}

/// Empty / missing `scope_json` → error when role is scope (must configure keys).
pub fn parse_scope_json(value: &Value) -> Result<ProjectScopeConfig, String> {
    if value.is_null() {
        return Err("scope_json required when project_role=scope".into());
    }
    let obj = value
        .as_object()
        .ok_or_else(|| "scope_json must be a JSON object".to_string())?;
    let keys_val = obj
        .get("scopeKeys")
        .ok_or_else(|| "scope_json.scopeKeys required".to_string())?;
    let arr = keys_val
        .as_array()
        .ok_or_else(|| "scope_json.scopeKeys must be a non-empty string array".to_string())?;
    if arr.is_empty() {
        return Err("scope_json.scopeKeys must be a non-empty string array".into());
    }
    let mut scope_keys = Vec::new();
    for (i, item) in arr.iter().enumerate() {
        let key = item
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("scope_json.scopeKeys[{i}] must be a non-empty string"))?;
        validate_field_key(key).map_err(|e| format!("scope_json.scopeKeys[{i}]: {e}"))?;
        if !scope_keys.iter().any(|k| k == key) {
            scope_keys.push(key.to_string());
        }
    }
    let idle_sleep_secs = match obj.get("idleSleepSecs") {
        None | Some(Value::Null) => DEFAULT_IDLE_SLEEP_SECS,
        Some(v) => {
            let n = v
                .as_u64()
                .ok_or_else(|| "scope_json.idleSleepSecs must be a positive integer".to_string())?;
            if n == 0 {
                return Err("scope_json.idleSleepSecs must be >= 1".into());
            }
            n
        }
    };
    Ok(ProjectScopeConfig {
        scope_keys,
        idle_sleep_secs,
    })
}

/// Validate `scope_json` against declared extraSession field names.
pub fn validate_scope_json_against_fields(
    scope_json: &Value,
    extra_session_fields_json: &Value,
) -> Result<ProjectScopeConfig, String> {
    let cfg = parse_scope_json(scope_json)?;
    let fields = parse_extra_session_fields_json(extra_session_fields_json)?;
    let allowed: BTreeSet<&str> = fields.iter().map(String::as_str).collect();
    for key in &cfg.scope_keys {
        if !allowed.contains(key.as_str()) {
            return Err(format!(
                "scope_json.scopeKeys contains {key} which is not in extraSessionFieldsJson"
            ));
        }
    }
    Ok(cfg)
}

/// Build canonical scope key from extraSession values (ordered by `scope_keys`).
pub fn build_scope_key(
    scope_keys: &[String],
    extra_session: Option<&Value>,
) -> Result<String, String> {
    let obj = extra_session
        .and_then(Value::as_object)
        .ok_or_else(|| "extraSession must be a JSON object for scope workers".to_string())?;
    let mut parts = Vec::with_capacity(scope_keys.len());
    for key in scope_keys {
        let raw = obj
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("extraSession.{key} required and must be a non-empty string"))?;
        parts.push(format!("{key}={}", escape_scope_value(raw)));
    }
    Ok(parts.join("\u{1f}"))
}

fn escape_scope_value(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\u{1f}', "\\x1f")
        .replace('=', "\\=")
}

/// Extract ordered scope key → value map used for worker identity / MCP bind snapshot.
pub fn scope_bind_values(
    scope_keys: &[String],
    extra_session: Option<&Value>,
) -> Result<BTreeMap<String, String>, String> {
    let obj = extra_session
        .and_then(Value::as_object)
        .ok_or_else(|| "extraSession must be a JSON object for scope workers".to_string())?;
    let mut out = BTreeMap::new();
    for key in scope_keys {
        let raw = obj
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("extraSession.{key} required and must be a non-empty string"))?;
        out.insert(key.clone(), raw.to_string());
    }
    Ok(out)
}

/// Values for MCP `${key}` substitution from `extraSession`.
///
/// Every placeholder in `template` must be a non-empty string field. Distinct from
/// [`scope_bind_values`]: scope keys identify the worker; template keys (e.g. `userToken`)
/// may be a strict superset and must still resolve. Author: kejiqing
pub fn mcp_template_render_values(
    template: &Value,
    extra_session: Option<&Value>,
) -> Result<BTreeMap<String, String>, String> {
    let obj = extra_session
        .and_then(Value::as_object)
        .ok_or_else(|| "extraSession must be a JSON object for scope workers".to_string())?;
    let mut out = BTreeMap::new();
    for key in collect_mcp_template_keys(template) {
        let raw = obj
            .get(&key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                format!("extraSession.{key} required for MCP template placeholder ${{{key}}}")
            })?;
        out.insert(key, raw.to_string());
    }
    Ok(out)
}

/// Render MCP servers JSON: substitute every `${…}` from `extraSession`. Author: kejiqing
pub fn render_mcp_servers_from_extra_session(
    template: &Value,
    extra_session: Option<&Value>,
) -> Result<Value, String> {
    let values = mcp_template_render_values(template, extra_session)?;
    Ok(render_mcp_template(template, &values))
}

#[must_use]
pub fn mcp_bind_is_initialized(stored: &Value) -> bool {
    stored
        .get("values")
        .and_then(Value::as_object)
        .is_some_and(|m| !m.is_empty())
}

#[must_use]
pub fn mcp_bind_snapshot(values: &BTreeMap<String, String>, mcp_servers: &Value) -> Value {
    let mut map = Map::new();
    for (k, v) in values {
        map.insert(k.clone(), Value::String(v.clone()));
    }
    json!({
        "values": Value::Object(map),
        "mcpServers": mcp_servers,
    })
}

/// Replace `${key}` in all string leaves using `values`. Unknown placeholders left unchanged
/// only after validation; call [`validate_mcp_template_placeholders`] on Admin save.
#[must_use]
pub fn render_mcp_template(value: &Value, values: &BTreeMap<String, String>) -> Value {
    match value {
        Value::String(s) => Value::String(render_template_string(s, values)),
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| render_mcp_template(v, values)).collect())
        }
        Value::Object(obj) => {
            let mut out = Map::new();
            for (k, v) in obj {
                out.insert(k.clone(), render_mcp_template(v, values));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

fn render_template_string(s: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = s.to_string();
    for (key, val) in values {
        let needle = format!("${{{key}}}");
        out = out.replace(&needle, val);
    }
    out
}

/// Collect `${name}` placeholders from MCP JSON strings.
#[must_use]
pub fn collect_mcp_template_keys(value: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk_collect_placeholders(value, &mut out);
    out
}

fn walk_collect_placeholders(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(s) => {
            let bytes = s.as_bytes();
            let mut i = 0;
            while i + 1 < bytes.len() {
                if bytes[i] == b'$' && bytes[i + 1] == b'{' {
                    if let Some(end) = s[i + 2..].find('}') {
                        let name = &s[i + 2..i + 2 + end];
                        if !name.is_empty()
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        {
                            out.insert(name.to_string());
                        }
                        i += 2 + end + 1;
                        continue;
                    }
                }
                i += 1;
            }
        }
        Value::Array(arr) => {
            for v in arr {
                walk_collect_placeholders(v, out);
            }
        }
        Value::Object(obj) => {
            for v in obj.values() {
                walk_collect_placeholders(v, out);
            }
        }
        _ => {}
    }
}

/// Admin save: every `${key}` must be in extraSession field list.
pub fn validate_mcp_template_placeholders(
    mcp_servers_json: &Value,
    extra_session_fields_json: &Value,
) -> Result<(), String> {
    let fields = parse_extra_session_fields_json(extra_session_fields_json)?;
    let allowed: BTreeSet<&str> = fields.iter().map(String::as_str).collect();
    let used = collect_mcp_template_keys(mcp_servers_json);
    for key in used {
        if !allowed.contains(key.as_str()) {
            return Err(format!(
                "mcpServersJson placeholder ${{{key}}} is not in extraSessionFieldsJson"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_scope_json_ok() {
        let cfg = parse_scope_json(&json!({
            "scopeKeys": ["tenant", "uid"],
            "idleSleepSecs": 60
        }))
        .unwrap();
        assert_eq!(cfg.scope_keys, vec!["tenant", "uid"]);
        assert_eq!(cfg.idle_sleep_secs, 60);
    }

    #[test]
    fn build_scope_key_ordered() {
        let keys = vec!["tenant".into(), "uid".into()];
        let k = build_scope_key(&keys, Some(&json!({"uid": "42", "tenant": "acme"}))).unwrap();
        assert_eq!(k, "tenant=acme\u{1f}uid=42");
    }

    #[test]
    fn render_replaces_placeholders() {
        let mut vals = BTreeMap::new();
        vals.insert("tenant".into(), "acme".into());
        let out = render_mcp_template(&json!({"url": "https://x/${tenant}/mcp"}), &vals);
        assert_eq!(out["url"], "https://x/acme/mcp");
    }

    #[test]
    fn template_render_values_include_non_scope_placeholders() {
        // Author: kejiqing — userToken is not a scopeKey but must still resolve.
        let template = json!({
            "mind-mcp": {
                "url": "https://mind.example/tenants/${tenantId}/mcp",
                "headers": {"Authorization": "Bearer ${userToken}"}
            }
        });
        let extra = json!({
            "tenantId": "t-1",
            "uid": "u-1",
            "userToken": "ut_secret"
        });
        let vals = mcp_template_render_values(&template, Some(&extra)).unwrap();
        assert_eq!(vals.get("tenantId").map(String::as_str), Some("t-1"));
        assert_eq!(vals.get("userToken").map(String::as_str), Some("ut_secret"));
        assert!(!vals.contains_key("uid"));

        let scope_keys = vec!["tenantId".into(), "uid".into()];
        let scope_vals = scope_bind_values(&scope_keys, Some(&extra)).unwrap();
        assert!(scope_vals.contains_key("uid"));
        assert!(!scope_vals.contains_key("userToken"));
    }

    #[test]
    fn render_from_extra_session_substitutes_user_token() {
        // Author: kejiqing — repro of proj_3024 mind-mcp 401 (literal Bearer ${userToken}).
        let template = json!({
            "mind-mcp": {
                "type": "streamable-http",
                "url": "https://mind.maxiot-inc.com/api/mind/tenants/${tenantId}/mcp",
                "headers": {"Authorization": "Bearer ${userToken}"}
            }
        });
        let extra = json!({
            "tenantId": "455785b1-1358-45ce-89c0-5a66e56d7826",
            "uid": "f60afc97-e1c7-4c1f-b600-51eea62db1d5",
            "userToken": "ut_mrMJZdwWU5Hgqy2J_test"
        });
        let out = render_mcp_servers_from_extra_session(&template, Some(&extra)).unwrap();
        assert_eq!(
            out["mind-mcp"]["url"],
            "https://mind.maxiot-inc.com/api/mind/tenants/455785b1-1358-45ce-89c0-5a66e56d7826/mcp"
        );
        assert_eq!(
            out["mind-mcp"]["headers"]["Authorization"],
            "Bearer ut_mrMJZdwWU5Hgqy2J_test"
        );
        assert!(
            !out["mind-mcp"]["headers"]["Authorization"]
                .as_str()
                .unwrap_or("")
                .contains("${"),
            "Authorization must not keep unresolved placeholders"
        );
    }

    #[test]
    fn template_render_values_require_placeholder_fields() {
        let template = json!({"headers": {"Authorization": "Bearer ${userToken}"}});
        let extra = json!({"tenantId": "t-1", "uid": "u-1"});
        let err = mcp_template_render_values(&template, Some(&extra)).unwrap_err();
        assert!(err.contains("userToken"));
    }
}
