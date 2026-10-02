//! Gateway task file loading and engine capability gate. Author: kejiqing

use std::path::Path;

use gateway_solve_turn::GatewaySolveTaskFile;

use crate::HarnessError;

/// Error prefix shared with the gateway-side capability gate.
pub const UNSUPPORTED_BY_ENGINE: &str = "unsupported_by_engine";

pub fn load_task(path: &Path) -> Result<GatewaySolveTaskFile, HarnessError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| HarnessError::new(400, format!("read task file {}: {e}", path.display())))?;
    serde_json::from_str(&raw)
        .map_err(|e| HarnessError::new(400, format!("parse task file {}: {e}", path.display())))
}

/// Plan mode, ask-user and sealed plans exist only in the claw engine.
pub fn reject_unsupported(task: &GatewaySolveTaskFile) -> Result<(), HarnessError> {
    let mode = task
        .interaction_mode
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    let field = if mode.eq_ignore_ascii_case("plan") {
        Some("interactionMode")
    } else if task.ask_user_question_enabled == Some(true) {
        Some("askUserQuestionEnabled")
    } else if task.sealed_plan_id.is_some() {
        Some("sealedPlanId")
    } else if task.sealed_plan_markdown.is_some() {
        Some("sealedPlanMarkdown")
    } else {
        None
    };
    match field {
        Some(f) => Err(HarnessError::new(
            400,
            format!("{UNSUPPORTED_BY_ENGINE}: {f}"),
        )),
        None => Ok(()),
    }
}

/// Wire model for this turn: task override, else `CLAW_DEFAULT_MODEL`.
pub fn resolve_model(task: &GatewaySolveTaskFile) -> Result<String, HarnessError> {
    task.model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("CLAW_DEFAULT_MODEL")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .ok_or_else(|| {
            HarnessError::internal("model missing: task.model and CLAW_DEFAULT_MODEL are empty")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task(extra: &serde_json::Value) -> GatewaySolveTaskFile {
        let mut base = json!({"requestId":"r1","userPrompt":"hi","turnId":"t1"});
        for (k, v) in extra.as_object().unwrap() {
            base[k] = v.clone();
        }
        serde_json::from_value(base).unwrap()
    }

    #[test]
    fn agent_mode_passes() {
        assert!(reject_unsupported(&task(
            &json!({"interactionMode":"agent","askUserQuestionEnabled":false})
        ))
        .is_ok());
        assert!(reject_unsupported(&task(&json!({}))).is_ok());
    }

    #[test]
    fn plan_ask_user_and_sealed_plan_are_rejected_with_400() {
        for (extra, field) in [
            (json!({"interactionMode":"plan"}), "interactionMode"),
            (
                json!({"askUserQuestionEnabled":true}),
                "askUserQuestionEnabled",
            ),
            (json!({"sealedPlanId":"p1"}), "sealedPlanId"),
            (json!({"sealedPlanMarkdown":"# p"}), "sealedPlanMarkdown"),
        ] {
            let err = reject_unsupported(&task(&extra)).unwrap_err();
            assert_eq!(err.status, 400);
            assert_eq!(err.message, format!("unsupported_by_engine: {field}"));
        }
    }
}
