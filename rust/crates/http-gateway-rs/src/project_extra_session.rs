//! Per-`proj_id` extraSession field list and solve-time validation. Author: kejiqing

use std::collections::BTreeMap;

use serde_json::Value;

/// Parse `extra_session_fields_json` from project_config (`string[]`).
pub fn parse_extra_session_fields_json(value: &Value) -> Result<Vec<String>, String> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    let arr = value
        .as_array()
        .ok_or_else(|| "extraSessionFieldsJson must be a JSON array".to_string())?;
    let mut out = Vec::new();
    for (i, item) in arr.iter().enumerate() {
        let key = item
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("extraSessionFieldsJson[{i}] must be a non-empty string"))?;
        validate_field_key(key).map_err(|e| format!("extraSessionFieldsJson[{i}]: {e}"))?;
        if !out.contains(&key.to_string()) {
            out.push(key.to_string());
        }
    }
    Ok(out)
}

pub fn validate_project_extra_session_fields_json(value: &Value) -> Result<(), String> {
    parse_extra_session_fields_json(value).map(|_| ())
}

fn validate_field_key(key: &str) -> Result<(), String> {
    if key.starts_with("_claw_") {
        return Err("field names must not use _claw_ prefix".to_string());
    }
    if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("field names must be ASCII alphanumeric or underscore".to_string());
    }
    Ok(())
}

/// Validates caller `extraSession` against ds field definitions (`defined_only` policy).
pub fn validate_extra_session_against_fields(
    extra_session: Option<&Value>,
    field_defs: &[String],
) -> Result<(), String> {
    if field_defs.is_empty() {
        return Ok(());
    }
    let Some(obj) = extra_session.and_then(Value::as_object) else {
        return Err("extraSession 不符合要求：必须为 JSON 对象".to_string());
    };
    for key in field_defs {
        match obj.get(key) {
            None => {
                return Err(format!("extraSession 不符合要求：缺少 {key}"));
            }
            Some(v) if !v.is_string() => {
                return Err(format!("extraSession 不符合要求：{key} 必须为字符串"));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Parse `?extraSession={"store_id":"x"}` for session list filter (only predefined keys).
pub fn parse_extra_session_search_filter(
    raw: Option<&Value>,
    allowed_fields: &[String],
) -> Result<BTreeMap<String, String>, String> {
    let Some(v) = raw else {
        return Ok(BTreeMap::new());
    };
    if v.is_null() {
        return Ok(BTreeMap::new());
    }
    let obj = v
        .as_object()
        .ok_or_else(|| "extraSession 筛选须为 JSON 对象".to_string())?;
    if obj.is_empty() {
        return Ok(BTreeMap::new());
    }
    let allowed: std::collections::BTreeSet<&str> =
        allowed_fields.iter().map(String::as_str).collect();
    let mut out = BTreeMap::new();
    for (key, val) in obj {
        if !allowed.contains(key.as_str()) {
            return Err(format!("extraSession 筛选含未定义字段: {key}"));
        }
        validate_field_key(key)?;
        let Some(s) = val.as_str() else {
            return Err(format!("extraSession 筛选 {key} 须为字符串"));
        };
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        out.insert(key.clone(), s.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_rejects_claw_prefix() {
        let err = parse_extra_session_fields_json(&json!(["_claw_x"])).unwrap_err();
        assert!(err.contains("_claw_"));
    }

    #[test]
    fn validate_requires_defined_keys() {
        let fields = vec!["store_id".to_string(), "org_id".to_string()];
        assert!(validate_extra_session_against_fields(None, &fields).is_err());
        let ok = json!({"store_id": "", "org_id": " x "});
        assert!(validate_extra_session_against_fields(Some(&ok), &fields).is_ok());
        let bad = json!({"store_id": "1"});
        assert!(validate_extra_session_against_fields(Some(&bad), &fields).is_err());
    }

    #[test]
    fn search_filter_only_allowed_keys() {
        let fields = vec!["store_id".to_string()];
        let ok =
            parse_extra_session_search_filter(Some(&json!({"store_id": "S1"})), &fields).unwrap();
        assert_eq!(ok.get("store_id").map(String::as_str), Some("S1"));
        assert!(parse_extra_session_search_filter(Some(&json!({"org_id": "x"})), &fields).is_err());
    }
}
