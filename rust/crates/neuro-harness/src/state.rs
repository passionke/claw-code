//! `.neuro-harness/state.json`: which engine owns this session and its ACP session id. Author: kejiqing

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{HarnessError, HARNESS_DIR};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessState {
    pub engine: String,
    pub agent_session_id: String,
}

#[must_use]
pub fn state_path(session_root: &Path) -> PathBuf {
    session_root.join(HARNESS_DIR).join("state.json")
}

/// `None` when this session has no ACP session yet. An engine mismatch is an error: the engine
/// is fixed at project creation.
pub fn read_state(session_root: &Path, engine: &str) -> Result<Option<HarnessState>, HarnessError> {
    let path = state_path(session_root);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(HarnessError::internal(format!(
                "read {}: {e}",
                path.display()
            )))
        }
    };
    let state: HarnessState = serde_json::from_str(&raw)
        .map_err(|e| HarnessError::internal(format!("parse {}: {e}", path.display())))?;
    if state.engine != engine {
        return Err(HarnessError::internal(format!(
            "session belongs to engine `{}`, this worker runs `{engine}`",
            state.engine
        )));
    }
    Ok(Some(state))
}

pub fn write_state(session_root: &Path, state: &HarnessState) -> Result<(), HarnessError> {
    let path = state_path(session_root);
    let body = serde_json::to_vec_pretty(state)
        .map_err(|e| HarnessError::internal(format!("serialize state: {e}")))?;
    std::fs::write(&path, body)
        .map_err(|e| HarnessError::internal(format!("write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_state_is_none_and_engine_mismatch_errors() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(HARNESS_DIR)).unwrap();
        assert_eq!(read_state(dir.path(), "opencode").unwrap(), None);
        let st = HarnessState {
            engine: "opencode".into(),
            agent_session_id: "ses_1".into(),
        };
        write_state(dir.path(), &st).unwrap();
        assert_eq!(read_state(dir.path(), "opencode").unwrap(), Some(st));
        let err = read_state(dir.path(), "appserver").unwrap_err();
        assert!(err.message.contains("engine `opencode`"));
    }
}
