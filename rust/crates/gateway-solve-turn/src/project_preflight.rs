//! Per-project solve preflight (code execution, not LSP). Truth: `project_config.solve_preflight_json`
//! in PostgreSQL; gateway materializes to `home/.claw/solve-preflight.json` on `ds_*` apply.
//! Author: kejiqing

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{DirectToolExecutor, GatewaySolveTurnError};
use runtime::Session;

/// Relative to `ds_*` root: `home/.claw/solve-preflight.json`.
pub const SOLVE_PREFLIGHT_CONFIG_REL: &str = "home/.claw/solve-preflight.json";

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct SolvePreflightConfig {
    /// Registered handler id, e.g. `sqlbot_mcp_start`, `none`.
    pub kind: String,
}

/// Validate `project_config.solve_preflight_json` before DB write.
pub fn validate_solve_preflight_json(value: &serde_json::Value) -> Result<(), String> {
    let cfg: SolvePreflightConfig =
        serde_json::from_value(value.clone()).map_err(|e| format!("solvePreflightJson: {e}"))?;
    match cfg.kind.as_str() {
        "none" | "sqlbot_mcp_start" => Ok(()),
        other => Err(format!(
            "solvePreflightJson.kind must be none or sqlbot_mcp_start, got {other:?}"
        )),
    }
}

fn ds_root_from_session_home(session_home: &Path) -> Option<PathBuf> {
    let sessions = session_home.parent()?;
    if sessions.file_name().and_then(|n| n.to_str()) != Some("sessions") {
        return None;
    }
    sessions.parent().map(Path::to_path_buf)
}

fn parse_solve_preflight_file(path: &Path) -> Option<SolvePreflightConfig> {
    let raw = std::fs::read_to_string(path).ok()?;
    let cfg: SolvePreflightConfig = serde_json::from_str(&raw).ok()?;
    if cfg.kind.trim().is_empty() || cfg.kind == "none" {
        return None;
    }
    Some(cfg)
}

fn resolve_solve_preflight_config(session_home: &Path) -> Option<SolvePreflightConfig> {
    // Pool worker: only the session dir is rw-mounted; preflight JSON is ro-mounted under guest home.
    let pool_mounted = session_home.join(SOLVE_PREFLIGHT_CONFIG_REL);
    if let Some(cfg) = parse_solve_preflight_file(&pool_mounted) {
        return Some(cfg);
    }
    let ds_root = ds_root_from_session_home(session_home)?;
    parse_solve_preflight_file(&ds_root.join(SOLVE_PREFLIGHT_CONFIG_REL))
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
    match cfg.kind.as_str() {
        "sqlbot_mcp_start" => {
            crate::sqlbot_preflight::run_sqlbot_preflight(session_home, session, executor)
        }
        other => Err(crate::err(
            crate::HTTP_INTERNAL,
            format!(
                "unknown solve-preflight kind {other:?} in {SOLVE_PREFLIGHT_CONFIG_REL} (registered: sqlbot_mcp_start, none)"
            ),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
        assert_eq!(cfg.kind, "sqlbot_mcp_start");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_config_under_ds_home() {
        let root = std::env::temp_dir().join(format!("claw-preflight-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session_home = root.join("sessions").join("sess1");
        fs::create_dir_all(session_home.join(".claw")).unwrap();
        let cfg_path = root.join(SOLVE_PREFLIGHT_CONFIG_REL);
        fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        fs::write(&cfg_path, r#"{"kind":"sqlbot_mcp_start"}"#).unwrap();
        let cfg = resolve_solve_preflight_config(&session_home).expect("config");
        assert_eq!(cfg.kind, "sqlbot_mcp_start");
        let _ = fs::remove_dir_all(&root);
    }
}
