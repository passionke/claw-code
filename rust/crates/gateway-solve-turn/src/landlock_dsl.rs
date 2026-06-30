//! Landlock allowlist DSL: system default + per-project override. Author: kejiqing

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// Resolved DSL source for audit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LandlockDslSource {
    SystemDefault,
    ProjectConfig,
}

/// Allowlist paths for strict per-solve Landlock jail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandlockDsl {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub rw: Vec<String>,
    #[serde(default)]
    pub ro: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

/// Factory seed for `gateway_global_settings.strictLandlockDefault` migration only.
#[must_use]
pub fn default_landlock_dsl() -> LandlockDsl {
    LandlockDsl {
        enabled: true,
        rw: vec![
            "${session_root}".into(),
            "${session_root}/.claw".into(),
            "${session_root}/work".into(),
            "${session_root}/.cache".into(),
            "${session_root}/tmp".into(),
        ],
        ro: vec![
            "/claw_ds/project_home_def".into(),
            "/usr".into(),
            "/bin".into(),
            "/lib".into(),
            "/lib64".into(),
            "/etc/ssl".into(),
            "/etc/resolv.conf".into(),
            "/etc/hosts".into(),
            "/etc/nsswitch.conf".into(),
        ],
    }
}

/// Variables gateway may substitute at solve time.
pub const LANDLOCK_DSL_VARIABLES: &[&str] = &[
    "${session_root}",
    "${project_home_def}",
    "${tmpdir}",
    "${claw_bin_dir}",
];

const FORBIDDEN_PREFIXES: &[&str] = &["/claw_sessions"];

/// Expand DSL variables using solve-time context.
#[derive(Debug, Clone)]
pub struct LandlockExpandContext<'a> {
    pub session_root: &'a str,
    pub project_home_def: &'a str,
    pub tmpdir: &'a str,
    pub claw_bin_dir: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLandlockPaths {
    pub source: LandlockDslSource,
    pub rw: Vec<String>,
    pub ro: Vec<String>,
}

/// Validate DSL shape and path safety (no variable expansion).
pub fn validate_landlock_dsl(dsl: &LandlockDsl) -> Result<(), String> {
    if !dsl.enabled {
        return Ok(());
    }
    if dsl.rw.is_empty() {
        return Err("landlock.rw must not be empty when enabled".into());
    }
    let has_session_root = dsl.rw.iter().any(|p| {
        let t = p.trim();
        t == "${session_root}" || t.starts_with("${session_root}/")
    });
    if !has_session_root {
        return Err("landlock.rw must include ${session_root}".into());
    }
    validate_path_list("rw", &dsl.rw)?;
    validate_path_list("ro", &dsl.ro)?;
    Ok(())
}

fn is_allowed_template_path(path: &str) -> bool {
    if path.starts_with('/') && !path.contains("${") {
        return true;
    }
    for var in LANDLOCK_DSL_VARIABLES {
        if path == *var || path.starts_with(&format!("{var}/")) {
            return true;
        }
    }
    false
}

fn validate_path_list(field: &str, paths: &[String]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for (idx, raw) in paths.iter().enumerate() {
        let path = raw.trim();
        if path.is_empty() {
            return Err(format!("landlock.{field}[{idx}] must not be empty"));
        }
        if path.contains("..") {
            return Err(format!("landlock.{field}[{idx}] must not contain '..'"));
        }
        if path.contains('*') || path.contains('?') {
            return Err(format!("landlock.{field}[{idx}] must not contain wildcards"));
        }
        if !is_allowed_template_path(path) {
            return Err(format!(
                "landlock.{field}[{idx}] must be absolute or use known variables, got {path:?}"
            ));
        }
        for prefix in FORBIDDEN_PREFIXES {
            if path == *prefix || path.starts_with(&format!("{prefix}/")) {
                return Err(format!(
                    "landlock.{field}[{idx}] must not allow {prefix} tree"
                ));
            }
        }
        if !seen.insert(path.to_string()) {
            return Err(format!("landlock.{field}[{idx}] duplicate path {path:?}"));
        }
    }
    Ok(())
}

/// Expand variables and normalize paths for Landlock rules.
pub fn expand_landlock_dsl(
    dsl: &LandlockDsl,
    source: LandlockDslSource,
    ctx: &LandlockExpandContext<'_>,
) -> Result<ResolvedLandlockPaths, String> {
    validate_landlock_dsl(dsl)?;
    if !dsl.enabled {
        return Err("expand_landlock_dsl called on disabled dsl".into());
    }
    Ok(ResolvedLandlockPaths {
        source,
        rw: expand_paths(&dsl.rw, ctx)?,
        ro: expand_paths(&dsl.ro, ctx)?,
    })
}

fn expand_paths(paths: &[String], ctx: &LandlockExpandContext<'_>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for raw in paths {
        let expanded = expand_one_path(raw.trim(), ctx)?;
        out.push(expanded);
    }
    Ok(out)
}

fn expand_one_path(path: &str, ctx: &LandlockExpandContext<'_>) -> Result<String, String> {
    let mut s = path.to_string();
    for var in LANDLOCK_DSL_VARIABLES {
        let value = match *var {
            "${session_root}" => ctx.session_root,
            "${project_home_def}" => ctx.project_home_def,
            "${tmpdir}" => ctx.tmpdir,
            "${claw_bin_dir}" => ctx.claw_bin_dir,
            _ => continue,
        };
        s = s.replace(var, value);
    }
    if s.contains("${") {
        return Err(format!("unexpanded variable in path {path:?}"));
    }
    if !s.starts_with('/') {
        return Err(format!("expanded path must be absolute, got {s:?}"));
    }
    if s.contains("..") {
        return Err(format!("expanded path must not contain '..', got {s:?}"));
    }
    Ok(s)
}

/// Parse `strict.landlock` from project `worker_profile_json`.
pub fn landlock_from_worker_profile_strict(strict: &Value) -> Option<LandlockDsl> {
    let use_system = strict
        .get("useSystemDefault")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if use_system {
        return None;
    }
    strict
        .get("landlock")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

/// Whether project worker profile requests its own landlock block.
#[must_use]
pub fn project_has_custom_landlock(worker_profile_json: &Value) -> bool {
    worker_profile_json
        .get("strict")
        .and_then(landlock_from_worker_profile_strict)
        .is_some()
}

/// Resolve effective DSL for a strict project (system default vs project override).
pub fn resolve_landlock_dsl(
    worker_profile_json: &Value,
    system_default: &LandlockDsl,
) -> Result<Option<(LandlockDsl, LandlockDslSource)>, String> {
    let mode = worker_profile_json
        .get("mode")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("strict")
        .to_ascii_lowercase();
    if mode == "relaxed" {
        return Ok(None);
    }
    let strict = worker_profile_json.get("strict");
    let (dsl, source) = if let Some(strict_val) = strict {
        if let Some(project) = landlock_from_worker_profile_strict(strict_val) {
            validate_landlock_dsl(&project)?;
            (project, LandlockDslSource::ProjectConfig)
        } else {
            validate_landlock_dsl(system_default)?;
            (system_default.clone(), LandlockDslSource::SystemDefault)
        }
    } else {
        validate_landlock_dsl(system_default)?;
        (system_default.clone(), LandlockDslSource::SystemDefault)
    };
    if !dsl.enabled {
        return Ok(None);
    }
    Ok(Some((dsl, source)))
}

/// Parse system `strictLandlockDefault` from global settings JSON value.
pub fn landlock_from_global_settings(settings_json: &Value) -> LandlockDsl {
    settings_json
        .get("strictLandlockDefault")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_else(default_landlock_dsl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_dsl_validates() {
        validate_landlock_dsl(&default_landlock_dsl()).unwrap();
    }

    #[test]
    fn rejects_claw_sessions_in_ro() {
        let mut dsl = default_landlock_dsl();
        dsl.ro.push("/claw_sessions".into());
        assert!(validate_landlock_dsl(&dsl).is_err());
    }

    #[test]
    fn resolve_inherits_system_when_strict_missing_landlock() {
        let profile = json!({"mode": "strict"});
        let sys = default_landlock_dsl();
        let resolved = resolve_landlock_dsl(&profile, &sys).unwrap().unwrap();
        assert_eq!(resolved.1, LandlockDslSource::SystemDefault);
    }

    #[test]
    fn resolve_uses_project_override() {
        let profile = json!({
            "mode": "strict",
            "strict": {
                "useSystemDefault": false,
                "landlock": {
                    "enabled": true,
                    "rw": ["${session_root}"],
                    "ro": ["/usr"]
                }
            }
        });
        let sys = default_landlock_dsl();
        let resolved = resolve_landlock_dsl(&profile, &sys).unwrap().unwrap();
        assert_eq!(resolved.1, LandlockDslSource::ProjectConfig);
        assert_eq!(resolved.0.ro, vec!["/usr".to_string()]);
    }

    #[test]
    fn expand_substitutes_session_root() {
        let dsl = LandlockDsl {
            enabled: true,
            rw: vec!["${session_root}/work".into()],
            ro: vec![],
        };
        let ctx = LandlockExpandContext {
            session_root: "/claw_sessions/seg1",
            project_home_def: "/claw_ds/project_home_def",
            tmpdir: "/claw_sessions/seg1/tmp",
            claw_bin_dir: "/usr/local/bin",
        };
        let expanded = expand_landlock_dsl(&dsl, LandlockDslSource::SystemDefault, &ctx).unwrap();
        assert_eq!(expanded.rw, vec!["/claw_sessions/seg1/work".to_string()]);
    }
}
