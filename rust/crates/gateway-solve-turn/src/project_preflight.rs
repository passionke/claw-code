//! Per-project solve preflight config resolution. Truth: `project_config.solve_preflight_json`
//! in PostgreSQL; gateway materializes to `home/.claw/solve-preflight.json` on `ds_*` apply.
//! Author: kejiqing

use std::path::Path;

use preflight_spi::{
    has_enabled_pipeline, materialize_pipeline_json, normalize_pipeline_steps,
    parse_pipeline_value, validate_pipeline_value, PreflightPipelineConfig, PreflightStep,
};
use serde_json::Value;

use crate::preflight_runner::{
    resolve_pipeline_steps_for_run, run_preflight_pipeline, session_first_turn_preflight_satisfied,
    PreflightRunParams, PreflightRunReport,
};
use crate::GatewaySolveTurnError;
use runtime::Session;

/// Relative to `ds_*` root: `home/.claw/solve-preflight.json`.
pub const SOLVE_PREFLIGHT_CONFIG_REL: &str = "home/.claw/solve-preflight.json";

/// Validate `project_config.solve_preflight_json` before DB write.
pub fn validate_solve_preflight_json(value: &serde_json::Value) -> Result<(), String> {
    validate_pipeline_value(value)
}

/// Persisted format under `home/.claw/solve-preflight.json`.
#[must_use]
pub fn materialize_solve_preflight_json(value: &Value) -> Value {
    materialize_pipeline_json(value)
}

#[must_use]
pub fn has_enabled_solve_preflight(value: &Value) -> bool {
    has_enabled_pipeline(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SolvePreflightFileState {
    Missing,
    Disabled,
    Enabled(PreflightPipelineConfig),
}

/// Resolved preflight source for one solve turn. Author: kejiqing
#[derive(Debug, Clone, PartialEq, Eq)]
enum SolvePreflightResolve {
    /// Explicit tombstone (`steps: []` / `kind: none`); run nothing.
    Disabled,
    /// No project preflight file; default runtime pipeline (`turn_language` only).
    Missing,
    Enabled(PreflightPipelineConfig),
}

fn read_solve_preflight_file_state(path: &Path) -> SolvePreflightFileState {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return SolvePreflightFileState::Missing;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return SolvePreflightFileState::Missing;
    };
    let Ok(cfg) = parse_pipeline_value(&value) else {
        return SolvePreflightFileState::Missing;
    };
    if normalize_pipeline_steps(&cfg).is_empty() {
        SolvePreflightFileState::Disabled
    } else {
        SolvePreflightFileState::Enabled(cfg)
    }
}

fn resolve_solve_preflight_state(session_home: &Path) -> SolvePreflightResolve {
    let session_mounted = session_home.join(SOLVE_PREFLIGHT_CONFIG_REL);
    match read_solve_preflight_file_state(&session_mounted) {
        SolvePreflightFileState::Disabled => return SolvePreflightResolve::Disabled,
        SolvePreflightFileState::Enabled(cfg) => return SolvePreflightResolve::Enabled(cfg),
        SolvePreflightFileState::Missing => {}
    }
    let config_root = runtime::gateway_project_config_root(session_home);
    match read_solve_preflight_file_state(&config_root.join(SOLVE_PREFLIGHT_CONFIG_REL)) {
        SolvePreflightFileState::Enabled(cfg) => SolvePreflightResolve::Enabled(cfg),
        SolvePreflightFileState::Disabled => SolvePreflightResolve::Disabled,
        SolvePreflightFileState::Missing => SolvePreflightResolve::Missing,
    }
}

/// Session-local marker wins; empty pipeline blocks fallback to stale `/claw_ds` files.
pub fn resolve_solve_preflight_config(session_home: &Path) -> Option<PreflightPipelineConfig> {
    match resolve_solve_preflight_state(session_home) {
        SolvePreflightResolve::Enabled(cfg) => Some(cfg),
        SolvePreflightResolve::Disabled | SolvePreflightResolve::Missing => None,
    }
}

/// Whether session-first-turn preflight steps are already reflected in the session transcript.
#[allow(dead_code)] // unit tests in `project_preflight`
pub(crate) fn preflight_satisfied(session_home: &Path, session: &Session) -> bool {
    let Some(cfg) = resolve_solve_preflight_config(session_home) else {
        return true;
    };
    let steps = normalize_pipeline_steps(&cfg);
    session_first_turn_preflight_satisfied(session_home, session, &steps)
}

/// Executable preflight steps for this turn (resolve + optional default fallback; does not run handlers).
#[allow(dead_code)] // unit tests in `project_preflight`
#[must_use]
pub(crate) fn plan_solve_preflight_steps(
    session_home: &Path,
    language_pipeline_json: &Value,
) -> Vec<PreflightStep> {
    let empty = PreflightPipelineConfig {
        steps: vec![],
        kinds: vec![],
    };
    match resolve_solve_preflight_state(session_home) {
        SolvePreflightResolve::Enabled(cfg) => {
            resolve_pipeline_steps_for_run(&cfg, language_pipeline_json, false)
        }
        SolvePreflightResolve::Disabled => {
            resolve_pipeline_steps_for_run(&empty, language_pipeline_json, false)
        }
        SolvePreflightResolve::Missing => {
            resolve_pipeline_steps_for_run(&empty, language_pipeline_json, true)
        }
    }
}

/// Run preflight pipeline for this turn (after user message is in session).
pub(crate) fn run_solve_preflight(
    params: PreflightRunParams<'_>,
    language_pipeline_json: &Value,
) -> Result<PreflightRunReport, GatewaySolveTurnError> {
    let empty = PreflightPipelineConfig {
        steps: vec![],
        kinds: vec![],
    };
    match resolve_solve_preflight_state(params.session_home) {
        SolvePreflightResolve::Enabled(cfg) => {
            run_preflight_pipeline(&cfg, language_pipeline_json, params, false)
        }
        SolvePreflightResolve::Disabled => {
            run_preflight_pipeline(&empty, language_pipeline_json, params, false)
        }
        SolvePreflightResolve::Missing => {
            run_preflight_pipeline(&empty, language_pipeline_json, params, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use preflight_spi::{PreflightScope, BUILTIN_SQLBOT_MCP_START, BUILTIN_TURN_LANGUAGE};
    use serde_json::json;
    use std::fs;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sample_language_pipeline_json() -> Value {
        json!({
            "languageInferencePriorMaxChars": 3000,
            "languageInferencePriorTurns": 5,
            "languageInferencePrompt": "Determine the language..."
        })
    }

    fn planned_plugin_ids(session_home: &Path, language_pipeline_json: &Value) -> Vec<String> {
        plan_solve_preflight_steps(session_home, language_pipeline_json)
            .into_iter()
            .map(|step| step.plugin_id)
            .collect()
    }

    /// Regression: Admin `kind:none` / `steps:[]` must not silently re-enable `turn_language`.
    #[test]
    fn regression_disabled_preflight_must_not_plan_turn_language() {
        let root =
            std::env::temp_dir().join(format!("claw-preflight-regress-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess-regress");
        fs::create_dir_all(session_home.join("home/.claw")).unwrap();
        // Same shape as project1 after removing all preflight steps in Admin.
        fs::write(
            session_home.join(SOLVE_PREFLIGHT_CONFIG_REL),
            serde_json::to_string(&json!({"kind": "none", "steps": []})).unwrap(),
        )
        .unwrap();

        let planned = planned_plugin_ids(&session_home, &sample_language_pipeline_json());
        assert!(
            planned.is_empty(),
            "explicit tombstone must not fallback to turn_language; got {planned:?}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_preflight_file_plans_default_turn_language() {
        let _env = env_guard();
        let root =
            std::env::temp_dir().join(format!("claw-preflight-miss-plan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess-miss-plan");
        fs::create_dir_all(session_home.join("home")).unwrap();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", root.join("no-ds"));

        let planned = planned_plugin_ids(&session_home, &json!({}));
        assert_eq!(planned, vec![BUILTIN_TURN_LANGUAGE.to_string()]);

        if let Some(p) = prev {
            std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", p);
        } else {
            std::env::remove_var("CLAW_PROJECT_CONFIG_ROOT");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn session_tombstone_blocks_pool_sqlbot_and_language_fallback() {
        let _env = env_guard();
        let ds_root = std::env::temp_dir().join(format!(
            "claw-preflight-pool-tomb-plan-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&ds_root);
        let session_home = ds_root
            .parent()
            .unwrap()
            .join(format!("sess-pool-tomb-plan-{}", std::process::id()));
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
            serde_json::to_string(&tombstone).unwrap(),
        )
        .unwrap();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", &ds_root);

        let planned = planned_plugin_ids(&session_home, &sample_language_pipeline_json());
        assert!(
            planned.is_empty(),
            "session tombstone must override pool sqlbot and skip language fallback; got {planned:?}"
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
    fn pool_tombstone_alone_plans_nothing() {
        let _env = env_guard();
        let ds_root =
            std::env::temp_dir().join(format!("claw-preflight-ds-tomb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&ds_root);
        let session_home = ds_root
            .parent()
            .unwrap()
            .join(format!("sess-ds-tomb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&session_home);
        fs::create_dir_all(ds_root.join("home/.claw")).unwrap();
        fs::write(ds_root.join(SOLVE_PREFLIGHT_CONFIG_REL), r#"{"steps":[]}"#).unwrap();
        fs::create_dir_all(session_home.join("home")).unwrap();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", &ds_root);

        let planned = planned_plugin_ids(&session_home, &sample_language_pipeline_json());
        assert!(planned.is_empty());

        if let Some(p) = prev {
            std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", p);
        } else {
            std::env::remove_var("CLAW_PROJECT_CONFIG_ROOT");
        }
        let _ = fs::remove_dir_all(&ds_root);
        let _ = fs::remove_dir_all(&session_home);
    }

    #[test]
    fn explicit_steps_without_turn_language_are_not_augmented() {
        let root =
            std::env::temp_dir().join(format!("claw-preflight-explicit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess-explicit");
        fs::create_dir_all(session_home.join("home/.claw")).unwrap();
        fs::write(
            session_home.join(SOLVE_PREFLIGHT_CONFIG_REL),
            serde_json::to_string(&json!({
                "steps": [{
                    "pluginId": BUILTIN_SQLBOT_MCP_START,
                    "scope": "session_first_turn",
                    "impl": { "type": "builtin", "handler": BUILTIN_SQLBOT_MCP_START }
                }]
            }))
            .unwrap(),
        )
        .unwrap();

        let steps = plan_solve_preflight_steps(&session_home, &sample_language_pipeline_json());
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].plugin_id, BUILTIN_SQLBOT_MCP_START);
        assert_eq!(steps[0].scope, PreflightScope::SessionFirstTurn);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_kinds_still_auto_prepends_turn_language() {
        let root =
            std::env::temp_dir().join(format!("claw-preflight-kinds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess-kinds");
        fs::create_dir_all(session_home.join("home/.claw")).unwrap();
        fs::write(
            session_home.join(SOLVE_PREFLIGHT_CONFIG_REL),
            r#"{"kinds":["sqlbot_mcp_start"]}"#,
        )
        .unwrap();

        let planned = planned_plugin_ids(&session_home, &json!({}));
        assert_eq!(
            planned,
            vec![
                BUILTIN_TURN_LANGUAGE.to_string(),
                BUILTIN_SQLBOT_MCP_START.to_string()
            ]
        );

        let _ = fs::remove_dir_all(&root);
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
        let steps = normalize_pipeline_steps(&cfg);
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].plugin_id, BUILTIN_TURN_LANGUAGE);
        assert_eq!(steps[1].plugin_id, BUILTIN_SQLBOT_MCP_START);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn validate_and_materialize_multi_kinds() {
        let raw = json!({
            "kinds": ["sqlbot_mcp_start", "none", "sqlbot_mcp_start"]
        });
        validate_solve_preflight_json(&raw).expect("valid");
        let out = materialize_solve_preflight_json(&raw);
        let steps = out
            .get("steps")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(
            steps[0].get("pluginId").and_then(serde_json::Value::as_str),
            Some(BUILTIN_TURN_LANGUAGE)
        );
    }

    #[test]
    fn has_enabled_preflight_works_for_legacy_kind() {
        assert!(has_enabled_solve_preflight(
            &json!({"kind":"sqlbot_mcp_start"})
        ));
        assert!(!has_enabled_solve_preflight(&json!({"kind":"none"})));
        assert!(!has_enabled_solve_preflight(&json!({"steps":[]})));
    }

    #[test]
    fn materialize_kind_none_writes_empty_steps() {
        let out = materialize_solve_preflight_json(&json!({"kind": "none"}));
        assert_eq!(out, json!({"steps": []}));
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

    #[test]
    fn explicit_tombstone_resolves_disabled_not_missing() {
        let root = std::env::temp_dir().join(format!("claw-preflight-tomb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess-tomb");
        fs::create_dir_all(session_home.join("home/.claw")).unwrap();
        fs::write(
            session_home.join(SOLVE_PREFLIGHT_CONFIG_REL),
            r#"{"steps":[]}"#,
        )
        .unwrap();
        assert_eq!(
            resolve_solve_preflight_state(&session_home),
            SolvePreflightResolve::Disabled
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_preflight_file_resolves_missing() {
        let _env = env_guard();
        let root = std::env::temp_dir().join(format!("claw-preflight-miss-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess-miss");
        fs::create_dir_all(session_home.join("home")).unwrap();
        let prev = std::env::var("CLAW_PROJECT_CONFIG_ROOT").ok();
        std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", root.join("no-ds"));
        assert_eq!(
            resolve_solve_preflight_state(&session_home),
            SolvePreflightResolve::Missing
        );
        if let Some(p) = prev {
            std::env::set_var("CLAW_PROJECT_CONFIG_ROOT", p);
        } else {
            std::env::remove_var("CLAW_PROJECT_CONFIG_ROOT");
        }
        let _ = fs::remove_dir_all(&root);
    }

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
        assert_eq!(
            resolve_solve_preflight_state(&session_home),
            SolvePreflightResolve::Disabled
        );
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
}
