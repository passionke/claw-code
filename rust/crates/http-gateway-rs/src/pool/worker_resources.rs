//! Per-project e2b worker cpu/memory normalization + fingerprint. Author: kejiqing
//!
//! Prevents template explosion: cpu ceil to 1..=16; memory ceil to 256 MiB steps, max 16 GiB.

use serde_json::{json, Map, Value};

pub const CPU_MIN: u32 = 1;
pub const CPU_MAX: u32 = 16;
pub const MEMORY_QUANTUM_MB: u32 = 256;
pub const MEMORY_MIN_MB: u32 = MEMORY_QUANTUM_MB;
pub const MEMORY_MAX_MB: u32 = 16 * 1024; // 16384

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerResources {
    pub cpu_count: u32,
    pub memory_mb: u32,
}

impl WorkerResources {
    #[must_use]
    pub fn fingerprint(&self) -> String {
        format!("cpu{}-mem{}", self.cpu_count, self.memory_mb)
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "cpuCount": self.cpu_count,
            "memoryMb": self.memory_mb,
        })
    }
}

/// Ceiling-normalize raw cpu/memory; reject non-positive / non-finite / over-cap. Author: kejiqing
pub fn normalize_worker_resources(
    cpu_raw: f64,
    memory_mb_raw: f64,
) -> Result<WorkerResources, String> {
    let cpu_count = normalize_cpu(cpu_raw)?;
    let memory_mb = normalize_memory_mb(memory_mb_raw)?;
    Ok(WorkerResources {
        cpu_count,
        memory_mb,
    })
}

fn normalize_cpu(raw: f64) -> Result<u32, String> {
    if !raw.is_finite() {
        return Err("resources.cpuCount must be a finite number".into());
    }
    if raw <= 0.0 {
        return Err("resources.cpuCount must be > 0".into());
    }
    let ceil = raw.ceil();
    if ceil > f64::from(CPU_MAX) {
        return Err(format!(
            "resources.cpuCount must be <= {CPU_MAX} after ceil; got {raw} → {ceil}"
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = ceil as u32;
    if !(CPU_MIN..=CPU_MAX).contains(&n) {
        return Err(format!(
            "resources.cpuCount must be {CPU_MIN}..={CPU_MAX} after ceil; got {raw} → {n}"
        ));
    }
    Ok(n)
}

fn normalize_memory_mb(raw: f64) -> Result<u32, String> {
    if !raw.is_finite() {
        return Err("resources.memoryMb must be a finite number".into());
    }
    if raw <= 0.0 {
        return Err("resources.memoryMb must be > 0".into());
    }
    let q = f64::from(MEMORY_QUANTUM_MB);
    let ceil_steps = (raw / q).ceil();
    let ceil_mb = ceil_steps * q;
    if ceil_mb > f64::from(MEMORY_MAX_MB) {
        return Err(format!(
            "resources.memoryMb must be <= {MEMORY_MAX_MB} after ceil-to-{MEMORY_QUANTUM_MB}; got {raw} → {ceil_mb}"
        ));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = ceil_mb as u32;
    if !(MEMORY_MIN_MB..=MEMORY_MAX_MB).contains(&n) {
        return Err(format!(
            "resources.memoryMb must be {MEMORY_MIN_MB}..={MEMORY_MAX_MB} (quantum {MEMORY_QUANTUM_MB}); got {raw} → {n}"
        ));
    }
    Ok(n)
}

fn json_number_as_f64(v: &Value, field: &str) -> Result<f64, String> {
    match v {
        Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| format!("resources.{field} must be a finite number")),
        Value::Null => Err(format!("resources.{field} must be a finite number")),
        _ => Err(format!("resources.{field} must be a number")),
    }
}

/// Parse + normalize `resources` object. Absent/null → `Ok(None)`. Author: kejiqing
pub fn resources_from_json(value: &Value) -> Result<Option<WorkerResources>, String> {
    match value.get("resources") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(obj)) => {
            let cpu = obj
                .get("cpuCount")
                .ok_or_else(|| "resources.cpuCount required".to_string())?;
            let mem = obj
                .get("memoryMb")
                .ok_or_else(|| "resources.memoryMb required".to_string())?;
            let cpu_raw = json_number_as_f64(cpu, "cpuCount")?;
            let mem_raw = json_number_as_f64(mem, "memoryMb")?;
            Ok(Some(normalize_worker_resources(cpu_raw, mem_raw)?))
        }
        Some(_) => Err("resources must be an object".into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceTemplateBinding {
    pub template_id: String,
    pub build_id: Option<String>,
    pub fingerprint: String,
}

/// Parse `resourceTemplate` when present; empty/invalid shape → `None` (caller strips). Author: kejiqing
pub fn resource_template_from_json(value: &Value) -> Option<ResourceTemplateBinding> {
    let obj = value.get("resourceTemplate")?.as_object()?;
    let template_id = obj
        .get("templateId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let fingerprint = obj
        .get("fingerprint")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_string();
    let build_id = obj
        .get("buildId")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(ResourceTemplateBinding {
        template_id,
        build_id,
        fingerprint,
    })
}

/// Valid binding only when fingerprint equals normalized resources. Author: kejiqing
#[must_use]
pub fn effective_resource_template(value: &Value) -> Option<ResourceTemplateBinding> {
    let resources = resources_from_json(value).ok().flatten()?;
    let binding = resource_template_from_json(value)?;
    if binding.fingerprint == resources.fingerprint() {
        Some(binding)
    } else {
        None
    }
}

/// Alias used for e2b Template.build / create of a resource SKU. Author: kejiqing
#[must_use]
pub fn resource_template_alias(fingerprint: &str, relaxed: bool) -> String {
    if relaxed {
        format!("claw-worker-relaxed-r-{fingerprint}")
    } else {
        format!("claw-worker-r-{fingerprint}")
    }
}

/// Normalize `resources` in place; strip stale / orphan `resourceTemplate`. Author: kejiqing
///
/// - omit/null `resources` → drop `resourceTemplate` (reverse path)
/// - present `resources` → store ceil-normalized values; keep `resourceTemplate` only if fingerprint matches
pub fn sanitize_worker_profile_resources(value: &Value) -> Result<Value, String> {
    let mut out = value.clone();
    let map = out
        .as_object_mut()
        .ok_or_else(|| "worker_profile_json must be an object".to_string())?;

    match resources_from_json(value)? {
        None => {
            map.remove("resources");
            map.remove("resourceTemplate");
        }
        Some(norm) => {
            map.insert("resources".into(), norm.to_json());
            match resource_template_from_json(value) {
                Some(binding) if binding.fingerprint == norm.fingerprint() => {
                    let mut rt = Map::new();
                    rt.insert("templateId".into(), json!(binding.template_id));
                    if let Some(b) = binding.build_id {
                        rt.insert("buildId".into(), json!(b));
                    }
                    rt.insert("fingerprint".into(), json!(binding.fingerprint));
                    map.insert("resourceTemplate".into(), Value::Object(rt));
                }
                _ => {
                    map.remove("resourceTemplate");
                }
            }
        }
    }
    Ok(out)
}

/// Validate resources / resourceTemplate shape (after or before sanitize). Author: kejiqing
pub fn validate_worker_profile_resources(value: &Value) -> Result<(), String> {
    let _ = resources_from_json(value)?;
    if let Some(rt) = value.get("resourceTemplate") {
        if rt.is_null() {
            return Ok(());
        }
        let Some(obj) = rt.as_object() else {
            return Err("resourceTemplate must be an object".into());
        };
        let tid = obj
            .get("templateId")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let fp = obj
            .get("fingerprint")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if tid.is_none() || fp.is_none() {
            return Err("resourceTemplate requires non-empty templateId and fingerprint".into());
        }
        if let Some(b) = obj.get("buildId") {
            if !b.is_null() && !b.is_string() {
                return Err("resourceTemplate.buildId must be a string when set".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_ceil_cases() {
        assert_eq!(normalize_cpu(1.0).unwrap(), 1);
        assert_eq!(normalize_cpu(1.1).unwrap(), 2);
        assert_eq!(normalize_cpu(2.0).unwrap(), 2);
        assert_eq!(normalize_cpu(15.01).unwrap(), 16);
        assert_eq!(normalize_cpu(16.0).unwrap(), 16);
    }

    #[test]
    fn cpu_rejects() {
        assert!(normalize_cpu(0.0).is_err());
        assert!(normalize_cpu(-1.0).is_err());
        assert!(normalize_cpu(16.1).is_err());
        assert!(normalize_cpu(f64::NAN).is_err());
        assert!(normalize_cpu(f64::INFINITY).is_err());
    }

    #[test]
    fn mem_ceil_to_256() {
        assert_eq!(normalize_memory_mb(1.0).unwrap(), 256);
        assert_eq!(normalize_memory_mb(256.0).unwrap(), 256);
        assert_eq!(normalize_memory_mb(257.0).unwrap(), 512);
        assert_eq!(normalize_memory_mb(4096.0).unwrap(), 4096);
        assert_eq!(normalize_memory_mb(16384.0).unwrap(), 16384);
    }

    #[test]
    fn mem_rejects() {
        assert!(normalize_memory_mb(0.0).is_err());
        assert!(normalize_memory_mb(-1.0).is_err());
        assert!(normalize_memory_mb(16385.0).is_err());
        assert!(normalize_memory_mb(f64::NAN).is_err());
    }

    #[test]
    fn fingerprint_stable_across_ceil_inputs() {
        let a = normalize_worker_resources(1.1, 257.0).unwrap();
        let b = normalize_worker_resources(2.0, 512.0).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.fingerprint(), "cpu2-mem512");
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_eq!(
            resource_template_alias(&a.fingerprint(), false),
            "claw-worker-r-cpu2-mem512"
        );
        assert_eq!(
            resource_template_alias(&a.fingerprint(), true),
            "claw-worker-relaxed-r-cpu2-mem512"
        );
    }

    #[test]
    fn pointer_match_requires_equal_fingerprint() {
        let json = json!({
            "mode": "strict",
            "resources": {"cpuCount": 4, "memoryMb": 8192},
            "resourceTemplate": {
                "templateId": "tpl_ok",
                "buildId": "b1",
                "fingerprint": "cpu4-mem8192"
            }
        });
        let eff = effective_resource_template(&json).expect("matched");
        assert_eq!(eff.template_id, "tpl_ok");

        let stale = json!({
            "mode": "strict",
            "resources": {"cpuCount": 4, "memoryMb": 8192},
            "resourceTemplate": {
                "templateId": "tpl_old",
                "fingerprint": "cpu2-mem1024"
            }
        });
        assert!(effective_resource_template(&stale).is_none());
    }

    #[test]
    fn reverse_clear_resources_strips_resource_template() {
        let incoming = json!({
            "mode": "strict",
            "resourceTemplate": {
                "templateId": "tpl_x",
                "fingerprint": "cpu4-mem8192"
            }
        });
        let out = sanitize_worker_profile_resources(&incoming).unwrap();
        assert!(out.get("resources").is_none());
        assert!(out.get("resourceTemplate").is_none());
        assert!(effective_resource_template(&out).is_none());
    }

    #[test]
    fn reverse_null_resources_strips_resource_template() {
        let incoming = json!({
            "mode": "strict",
            "resources": null,
            "resourceTemplate": {
                "templateId": "tpl_x",
                "fingerprint": "cpu4-mem8192"
            }
        });
        let out = sanitize_worker_profile_resources(&incoming).unwrap();
        assert!(out.get("resources").is_none());
        assert!(out.get("resourceTemplate").is_none());
    }

    #[test]
    fn dirty_orphan_resource_template_stripped_when_resources_absent() {
        // Half-corrupt: no resources but leftover pointer — sanitize strips.
        let dirty = json!({
            "mode": "strict",
            "poolSize": 1,
            "resourceTemplate": {
                "templateId": "tpl_orphan",
                "fingerprint": "cpu8-mem4096"
            }
        });
        let out = sanitize_worker_profile_resources(&dirty).unwrap();
        assert_eq!(out.get("resourceTemplate"), None);
        assert!(effective_resource_template(&dirty).is_none());
        assert!(effective_resource_template(&out).is_none());
    }

    #[test]
    fn sanitize_normalizes_and_keeps_matching_pointer() {
        let incoming = json!({
            "mode": "strict",
            "resources": {"cpuCount": 1.1, "memoryMb": 257},
            "resourceTemplate": {
                "templateId": "tpl_a",
                "buildId": "b",
                "fingerprint": "cpu2-mem512"
            }
        });
        let out = sanitize_worker_profile_resources(&incoming).unwrap();
        assert_eq!(out["resources"]["cpuCount"], json!(2));
        assert_eq!(out["resources"]["memoryMb"], json!(512));
        assert_eq!(out["resourceTemplate"]["templateId"], json!("tpl_a"));
        assert!(effective_resource_template(&out).is_some());
    }

    #[test]
    fn sanitize_strips_mismatched_pointer_keeps_resources() {
        let incoming = json!({
            "mode": "strict",
            "resources": {"cpuCount": 4, "memoryMb": 1024},
            "resourceTemplate": {
                "templateId": "tpl_old",
                "fingerprint": "cpu2-mem512"
            }
        });
        let out = sanitize_worker_profile_resources(&incoming).unwrap();
        assert_eq!(out["resources"]["cpuCount"], json!(4));
        assert_eq!(out["resources"]["memoryMb"], json!(1024));
        assert!(out.get("resourceTemplate").is_none());
        assert!(effective_resource_template(&out).is_none());
    }

    #[test]
    fn never_configured_baseline_has_no_effective_template() {
        let baseline = json!({"mode": "strict"});
        let cleared = sanitize_worker_profile_resources(&json!({
            "mode": "strict",
            "resources": {"cpuCount": 2, "memoryMb": 512},
            "resourceTemplate": {"templateId": "t", "fingerprint": "cpu2-mem512"}
        }))
        .unwrap();
        let cleared = sanitize_worker_profile_resources(&json!({
            "mode": "strict",
            "resources": null,
            "resourceTemplate": cleared.get("resourceTemplate").cloned().unwrap_or(Value::Null)
        }))
        .unwrap();
        assert_eq!(
            effective_resource_template(&baseline).is_some(),
            effective_resource_template(&cleared).is_some()
        );
        assert!(cleared.get("resourceTemplate").is_none());
        assert_eq!(cleared["mode"], json!("strict"));
    }
}
