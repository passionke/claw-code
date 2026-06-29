//! Per-project solve preflight (code execution, not LSP). Truth: `project_config.solve_preflight_json`
//! in PostgreSQL; gateway materializes to `home/.claw/solve-preflight.json` on `ds_*` apply.
//! Author: kejiqing

use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::{DirectToolExecutor, GatewaySolveTurnError};
use runtime::Session;

/// Relative to `ds_*` root: `home/.claw/solve-preflight.json`.
pub const SOLVE_PREFLIGHT_CONFIG_REL: &str = "home/.claw/solve-preflight.json";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SolvePreflightConfig {
    /// Registered handler ids in execution order.
    #[serde(default)]
    pub kinds: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
struct LegacySolvePreflightConfig {
    pub kind: String,
}

fn normalize_kinds(raw: &[String]) -> Vec<String> {
    raw.iter()
        .map(|k| k.trim())
        .filter(|k| !k.is_empty() && *k != "none")
        .map(ToString::to_string)
        .collect()
}

fn parse_solve_preflight_value(value: &Value) -> Result<SolvePreflightConfig, String> {
    if value.get("kinds").is_some() {
        let cfg: SolvePreflightConfig = serde_json::from_value(value.clone())
            .map_err(|e| format!("solvePreflightJson: {e}"))?;
        return Ok(SolvePreflightConfig {
            kinds: normalize_kinds(&cfg.kinds),
        });
    }
    let legacy: LegacySolvePreflightConfig =
        serde_json::from_value(value.clone()).map_err(|e| format!("solvePreflightJson: {e}"))?;
    let kind = legacy.kind.trim();
    if kind.is_empty() || kind == "none" {
        return Ok(SolvePreflightConfig { kinds: vec![] });
    }
    Ok(SolvePreflightConfig {
        kinds: vec![kind.to_string()],
    })
}

/// Validate `project_config.solve_preflight_json` before DB write.
pub fn validate_solve_preflight_json(value: &serde_json::Value) -> Result<(), String> {
    let cfg = parse_solve_preflight_value(value)?;
    for kind in &cfg.kinds {
        match kind.as_str() {
            "sqlbot_mcp_start" => {}
            other => {
                return Err(format!(
                    "solvePreflightJson kinds must be sqlbot_mcp_start (or legacy kind=none), got {other:?}"
                ))
            }
        }
    }
    Ok(())
}

/// Persisted format under `home/.claw/solve-preflight.json`.
#[must_use]
pub fn materialize_solve_preflight_json(value: &Value) -> Value {
    let Ok(cfg) = parse_solve_preflight_value(value) else {
        return json!({ "kinds": [] });
    };
    json!({ "kinds": cfg.kinds })
}

#[must_use]
pub fn has_enabled_solve_preflight(value: &Value) -> bool {
    parse_solve_preflight_value(value)
        .map(|cfg| !cfg.kinds.is_empty())
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SolvePreflightFileState {
    /// Marker missing; caller may fall back to project config root.
    Missing,
    /// PG/materialize wrote `{"kinds":[]}` — preflight explicitly off for this solve.
    Disabled,
    Enabled(SolvePreflightConfig),
}

fn read_solve_preflight_file_state(path: &Path) -> SolvePreflightFileState {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return SolvePreflightFileState::Missing;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return SolvePreflightFileState::Missing;
    };
    let Ok(cfg) = parse_solve_preflight_value(&value) else {
        return SolvePreflightFileState::Missing;
    };
    if cfg.kinds.is_empty() {
        SolvePreflightFileState::Disabled
    } else {
        SolvePreflightFileState::Enabled(cfg)
    }
}

/// Session-local marker wins; empty `kinds` blocks fallback to stale `/claw_ds` files. Author: kejiqing
fn resolve_solve_preflight_config(session_home: &Path) -> Option<SolvePreflightConfig> {
    let session_mounted = session_home.join(SOLVE_PREFLIGHT_CONFIG_REL);
    match read_solve_preflight_file_state(&session_mounted) {
        SolvePreflightFileState::Disabled => return None,
        SolvePreflightFileState::Enabled(cfg) => return Some(cfg),
        SolvePreflightFileState::Missing => {}
    }
    let config_root = runtime::gateway_project_config_root(session_home);
    match read_solve_preflight_file_state(&config_root.join(SOLVE_PREFLIGHT_CONFIG_REL)) {
        SolvePreflightFileState::Enabled(cfg) => Some(cfg),
        SolvePreflightFileState::Disabled | SolvePreflightFileState::Missing => None,
    }
}

/// Whether configured preflight steps are already reflected in the session transcript.
pub(crate) fn preflight_satisfied(session_home: &Path, session: &Session) -> bool {
    let Some(cfg) = resolve_solve_preflight_config(session_home) else {
        return true;
    };
    for kind in &cfg.kinds {
        match kind.as_str() {
            "sqlbot_mcp_start" => {
                if crate::sqlbot_preflight::sqlbot_query_context_from_session(session).is_none() {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// First turn of a `sessionId` only (caller gates on missing jsonl). Runs project-defined preflight.
pub(crate) fn run_first_turn_preflight(
    session_home: &Path,
    session: &mut Session,
    executor: &mut DirectToolExecutor,
) -> Result<(), GatewaySolveTurnError> {
    let Some(cfg) = resolve_solve_preflight_config(session_home) else {
        return Ok(());
    };
    for kind in cfg.kinds {
        match kind.as_str() {
            "sqlbot_mcp_start" => {
                crate::sqlbot_preflight::run_sqlbot_preflight(session_home, session, executor)?;
            }
            other => {
                return Err(crate::err(
                    crate::HTTP_INTERNAL,
                    format!(
                        "unknown solve-preflight kind {other:?} in {SOLVE_PREFLIGHT_CONFIG_REL} (registered: sqlbot_mcp_start)"
                    ),
                ))
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    /// Serializes tests that read/write the process-global `CLAW_PROJECT_CONFIG_ROOT`
    /// env so parallel runners cannot leak it across cases. Author: kejiqing
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn resolve_config_from_pool_ro_mount_under_session_home() {
        let root = std::env::temp_dir().join(format!("claw-preflight-pool-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess-pool");
        fs::create_dir_all(session_home.join("home/.claw")).unwrap();
        fs::write(
            session_home.join(SOLVE_PREFLIGHT_CONFIG_REL),
            r#"{"kind":"sqlbot_mcp_start"}"#,
        )
        .unwrap();
        let cfg = resolve_solve_preflight_config(&session_home).expect("pool mount path");
        assert_eq!(cfg.kinds, vec!["sqlbot_mcp_start".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_config_from_claw_project_config_root_env() {
        let _env = env_guard();
        let ds_root =
            std::env::temp_dir().join(format!("claw-preflight-env-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ds_root);
        let session_home = ds_root
            .parent()
            .unwrap()
            .join(format!("sess-env-{}", std::process::id()));
        let _ = fs::remove_dir_all(&session_home);
        fs::create_dir_all(session_home.join(".claw")).unwrap();
        let cfg_path = ds_root.join(SOLVE_PREFLIGHT_CONFIG_REL);
        fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        fs::write(&cfg_path, r#"{"kind":"sqlbot_mcp_start"}"#).unwrap();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", &ds_root);
        let cfg = resolve_solve_preflight_config(&session_home).expect("env config root");
        if let Some(p) = prev {
            std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", p);
        } else {
            std::env::remove_var("CLAW_PROJECT_CONFIG_ROOT");
        }
        assert_eq!(cfg.kinds, vec!["sqlbot_mcp_start".to_string()]);
        let _ = fs::remove_dir_all(&ds_root);
        let _ = fs::remove_dir_all(&session_home);
    }

    #[test]
    fn resolve_config_under_ds_home() {
        let _env = env_guard();
        let root = std::env::temp_dir().join(format!("claw-preflight-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess1");
        fs::create_dir_all(session_home.join(".claw")).unwrap();
        let cfg_path = root.join(SOLVE_PREFLIGHT_CONFIG_REL);
        fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        fs::write(&cfg_path, r#"{"kind":"sqlbot_mcp_start"}"#).unwrap();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::remove_var("CLAW_PROJECT_CONFIG_ROOT");
        let cfg = resolve_solve_preflight_config(&session_home).expect("config");
        if let Some(p) = prev {
            std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", p);
        }
        assert_eq!(cfg.kinds, vec!["sqlbot_mcp_start".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn validate_and_materialize_multi_kinds() {
        let raw = json!({
            "kinds": ["sqlbot_mcp_start", "none", "sqlbot_mcp_start"]
        });
        validate_solve_preflight_json(&raw).expect("valid");
        assert_eq!(
            materialize_solve_preflight_json(&raw),
            json!({"kinds": ["sqlbot_mcp_start", "sqlbot_mcp_start"]})
        );
    }

    #[test]
    fn has_enabled_preflight_works_for_legacy_kind() {
        assert!(has_enabled_solve_preflight(
            &json!({"kind":"sqlbot_mcp_start"})
        ));
        assert!(!has_enabled_solve_preflight(&json!({"kind":"none"})));
        assert!(!has_enabled_solve_preflight(&json!({"kinds":[]})));
    }

    #[test]
    fn materialize_kind_none_writes_empty_kinds_array() {
        assert_eq!(
            materialize_solve_preflight_json(&json!({"kind": "none"})),
            json!({"kinds": []})
        );
        assert_eq!(
            materialize_solve_preflight_json(&json!({"kinds": ["none"]})),
            json!({"kinds": []})
        );
    }

    #[test]
    fn materialize_legacy_sqlbot_kind() {
        assert_eq!(
            materialize_solve_preflight_json(&json!({"kind": "sqlbot_mcp_start"})),
            json!({"kinds": ["sqlbot_mcp_start"]})
        );
    }

    #[test]
    fn resolve_config_root_disabled_tombstone_returns_none() {
        let _env = env_guard();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::remove_var("CLAW_PROJECT_CONFIG_ROOT");
        let root = std::env::temp_dir().join(format!("claw-preflight-off-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess-off");
        fs::create_dir_all(session_home.join(".claw")).unwrap();
        let cfg_path = root.join(SOLVE_PREFLIGHT_CONFIG_REL);
        fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        fs::write(&cfg_path, r#"{"kinds":[]}"#).unwrap();
        let resolved = resolve_solve_preflight_config(&session_home);
        if let Some(p) = prev {
            std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", p);
        }
        let _ = fs::remove_dir_all(&root);
        assert!(
            resolved.is_none(),
            "empty kinds on config root must not enable preflight"
        );
    }

    #[test]
    fn resolve_session_enabled_wins_over_config_root() {
        let root = std::env::temp_dir().join(format!("claw-preflight-pri-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess-pri");
        fs::create_dir_all(session_home.join("home/.claw")).unwrap();
        fs::write(
            session_home.join(SOLVE_PREFLIGHT_CONFIG_REL),
            r#"{"kinds":["sqlbot_mcp_start"]}"#,
        )
        .unwrap();
        let cfg_path = root.join(SOLVE_PREFLIGHT_CONFIG_REL);
        fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        fs::write(&cfg_path, r#"{"kinds":[]}"#).unwrap();
        let cfg = resolve_solve_preflight_config(&session_home).expect("session wins");
        assert_eq!(cfg.kinds, vec!["sqlbot_mcp_start".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }

    /// End-to-end pool fix: PG `kind:none` → materialized tombstone → no preflight despite stale `/claw_ds`.
    #[test]
    fn pool_pg_none_materialized_tombstone_skips_preflight() {
        let _env = env_guard();
        let ds_root =
            std::env::temp_dir().join(format!("claw-preflight-pool-fix-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ds_root);
        let session_home = ds_root
            .parent()
            .unwrap()
            .join(format!("sess-pool-fix-{}", std::process::id()));
        let _ = fs::remove_dir_all(&session_home);
        fs::create_dir_all(ds_root.join("home/.claw")).unwrap();
        fs::write(
            ds_root.join(SOLVE_PREFLIGHT_CONFIG_REL),
            r#"{"kind":"sqlbot_mcp_start"}"#,
        )
        .unwrap();
        fs::create_dir_all(session_home.join("home/.claw")).unwrap();
        let tombstone = materialize_solve_preflight_json(&json!({"kind": "none"}));
        fs::write(
            session_home.join(SOLVE_PREFLIGHT_CONFIG_REL),
            serde_json::to_string(&tombstone).expect("tombstone json"),
        )
        .unwrap();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", &ds_root);
        assert!(resolve_solve_preflight_config(&session_home).is_none());
        assert!(preflight_satisfied(&session_home, &Session::new()));
        if let Some(p) = prev {
            std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", p);
        } else {
            std::env::remove_var("CLAW_PROJECT_CONFIG_ROOT");
        }
        let _ = fs::remove_dir_all(&ds_root);
        let _ = fs::remove_dir_all(&session_home);
    }

    #[test]
    fn preflight_satisfied_without_sqlbot_context() {
        let root = std::env::temp_dir().join(format!("claw-preflight-sat-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess1");
        fs::create_dir_all(session_home.join("home/.claw")).unwrap();
        fs::write(
            session_home.join(SOLVE_PREFLIGHT_CONFIG_REL),
            r#"{"kinds":["sqlbot_mcp_start"]}"#,
        )
        .unwrap();
        let session = Session::new();
        assert!(!preflight_satisfied(&session_home, &session));
        let _ = fs::remove_dir_all(&root);
    }

    /// Pool bug: PG `kind:none` but stale `/claw_ds` marker still enabled preflight when session
    /// home was wiped each solve and `materialize_in` did not write a tombstone. Author: kejiqing
    #[test]
    fn stale_claw_ds_preflight_blocked_by_session_disabled_marker() {
        let _env = env_guard();
        let ds_root =
            std::env::temp_dir().join(format!("claw-preflight-stale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ds_root);
        let session_home = ds_root
            .parent()
            .unwrap()
            .join(format!("sess-stale-{}", std::process::id()));
        let _ = fs::remove_dir_all(&session_home);
        fs::create_dir_all(ds_root.join("home/.claw")).unwrap();
        fs::write(
            ds_root.join(SOLVE_PREFLIGHT_CONFIG_REL),
            r#"{"kind":"sqlbot_mcp_start"}"#,
        )
        .unwrap();
        fs::create_dir_all(session_home.join("home/.claw")).unwrap();
        fs::write(
            session_home.join(SOLVE_PREFLIGHT_CONFIG_REL),
            r#"{"kinds":[]}"#,
        )
        .unwrap();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", &ds_root);
        assert!(
            resolve_solve_preflight_config(&session_home).is_none(),
            "session tombstone must block stale /claw_ds preflight"
        );
        if let Some(p) = prev {
            std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", p);
        } else {
            std::env::remove_var("CLAW_PROJECT_CONFIG_ROOT");
        }
        let _ = fs::remove_dir_all(&ds_root);
        let _ = fs::remove_dir_all(&session_home);
    }

    #[test]
    fn stale_claw_ds_preflight_used_when_session_marker_missing() {
        let _env = env_guard();
        let ds_root =
            std::env::temp_dir().join(format!("claw-preflight-miss-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ds_root);
        let session_home = ds_root
            .parent()
            .unwrap()
            .join(format!("sess-miss-{}", std::process::id()));
        let _ = fs::remove_dir_all(&session_home);
        fs::create_dir_all(ds_root.join("home/.claw")).unwrap();
        fs::write(
            ds_root.join(SOLVE_PREFLIGHT_CONFIG_REL),
            r#"{"kind":"sqlbot_mcp_start"}"#,
        )
        .unwrap();
        fs::create_dir_all(session_home.join(".claw")).unwrap();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", &ds_root);
        let cfg = resolve_solve_preflight_config(&session_home)
            .expect("falls back to /claw_ds when session marker absent");
        assert_eq!(cfg.kinds, vec!["sqlbot_mcp_start".to_string()]);
        if let Some(p) = prev {
            std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", p);
        } else {
            std::env::remove_var("CLAW_PROJECT_CONFIG_ROOT");
        }
        let _ = fs::remove_dir_all(&ds_root);
        let _ = fs::remove_dir_all(&session_home);
    }
}
