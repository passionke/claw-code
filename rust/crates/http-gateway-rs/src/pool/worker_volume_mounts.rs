//! Project worker `volumeMounts` (e2b official Volume protocol). Author: kejiqing
//!
//! Stores logical volume **name** + guest **path** only. Physical CIFS/NFS binding
//! stays on e2b Admin; Gateway never stores UNC credentials.

use serde_json::{json, Value};

use claw_e2b_sandbox_client::E2bVolumeMount;

/// Parse `workerProfileJson.volumeMounts` → create-sandbox mounts. Author: kejiqing
pub fn volume_mounts_from_json(value: &Value) -> Result<Vec<E2bVolumeMount>, String> {
    let Some(raw) = value.get("volumeMounts") else {
        return Ok(Vec::new());
    };
    if raw.is_null() {
        return Ok(Vec::new());
    }
    let Some(arr) = raw.as_array() else {
        return Err("volumeMounts must be an array".into());
    };
    let mut out = Vec::with_capacity(arr.len());
    let mut seen_names = std::collections::BTreeSet::new();
    let mut seen_paths = std::collections::BTreeSet::new();
    for (i, item) in arr.iter().enumerate() {
        let obj = item
            .as_object()
            .ok_or_else(|| format!("volumeMounts[{i}] must be an object"))?;
        let name = obj
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("volumeMounts[{i}].name required"))?
            .to_string();
        let path = obj
            .get("path")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("volumeMounts[{i}].path required"))?
            .to_string();
        validate_guest_mount_path(&path).map_err(|e| format!("volumeMounts[{i}].path: {e}"))?;
        if !seen_names.insert(name.clone()) {
            return Err(format!("volumeMounts duplicate name={name}"));
        }
        if !seen_paths.insert(path.clone()) {
            return Err(format!("volumeMounts duplicate path={path}"));
        }
        out.push(E2bVolumeMount { name, path });
    }
    Ok(out)
}

fn validate_guest_mount_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err("must be an absolute guest path".into());
    }
    if path.contains("..") {
        return Err("must not contain ..".into());
    }
    if path == "/" {
        return Err("must not mount at /".into());
    }
    Ok(())
}

/// Normalize / strip empty `volumeMounts` on worker profile write. Author: kejiqing
pub fn sanitize_worker_profile_volume_mounts(value: &Value) -> Result<Value, String> {
    let mounts = volume_mounts_from_json(value)?;
    let mut out = value.clone();
    let map = out
        .as_object_mut()
        .ok_or_else(|| "worker_profile_json must be an object".to_string())?;
    if mounts.is_empty() {
        map.remove("volumeMounts");
    } else {
        let arr: Vec<Value> = mounts
            .into_iter()
            .map(|m| json!({ "name": m.name, "path": m.path }))
            .collect();
        map.insert("volumeMounts".into(), Value::Array(arr));
    }
    Ok(out)
}

pub fn validate_worker_profile_volume_mounts(value: &Value) -> Result<(), String> {
    let _ = volume_mounts_from_json(value)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_rejects_bad_path() {
        let ok = json!({
            "volumeMounts": [{ "name": "ivqa-source", "path": "/mnt/ivqa" }]
        });
        let m = volume_mounts_from_json(&ok).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].name, "ivqa-source");
        assert_eq!(m[0].path, "/mnt/ivqa");

        let bad = json!({
            "volumeMounts": [{ "name": "ivqa-source", "path": "mnt/ivqa" }]
        });
        assert!(volume_mounts_from_json(&bad).is_err());
    }

    #[test]
    fn sanitize_clears_empty() {
        let cleared = sanitize_worker_profile_volume_mounts(&json!({
            "mode": "strict",
            "volumeMounts": []
        }))
        .unwrap();
        assert!(cleared.get("volumeMounts").is_none());
    }
}
