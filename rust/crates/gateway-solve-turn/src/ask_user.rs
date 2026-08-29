//! AskUserQuestion HITL for gateway solve: file wait + stdout A2UI (no stdin). Author: kejiqing

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::gateway_stdout::emit_raw_json;
use crate::interaction_mode::InteractionMode;

pub const ASK_USER_QUESTION_TOOL_NAME: &str = "AskUserQuestion";
pub const ASK_USER_DIR_REL: &str = ".claw/ask-user";
pub const ASK_USER_EVENT: &str = "ask.user";
pub const ASK_USER_CLEARED_EVENT: &str = "ask.user.cleared";

const DEFAULT_POLL_MS: u64 = 250;
const DEFAULT_TIMEOUT_SECS: u64 = 30 * 60;

#[derive(Debug, Clone, Deserialize)]
pub struct AskUserQuestionInput {
    pub question: String,
    #[serde(default)]
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserPendingFile {
    pub question_id: String,
    pub turn_id: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
    pub a2ui: Value,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserAnswerFile {
    pub question_id: String,
    pub answer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
    pub answered_at_ms: i64,
}

/// Read `workerProfileJson.askUserQuestionInAgent` (default false). Author: kejiqing
#[must_use]
pub fn ask_user_question_in_agent_from_profile(profile: &Value) -> bool {
    profile
        .get("askUserQuestionInAgent")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Plan always on; Agent only when proj enables. Author: kejiqing
#[must_use]
pub fn resolve_ask_user_question_enabled(mode: InteractionMode, ask_in_agent: bool) -> bool {
    match mode {
        InteractionMode::Plan => true,
        InteractionMode::Agent => ask_in_agent,
    }
}

/// When the allowlist is non-empty, force-include or strip AskUserQuestion. Author: kejiqing
pub fn apply_ask_user_tool_gate(tools: &mut Vec<String>, enabled: bool) {
    if tools.is_empty() {
        return;
    }
    tools.retain(|t| t != ASK_USER_QUESTION_TOOL_NAME);
    if enabled {
        tools.push(ASK_USER_QUESTION_TOOL_NAME.to_string());
    }
}

#[must_use]
pub fn ask_user_dir(session_home: &Path) -> PathBuf {
    session_home.join(ASK_USER_DIR_REL)
}

#[must_use]
pub fn pending_path(session_home: &Path, question_id: &str) -> PathBuf {
    ask_user_dir(session_home).join(format!("{question_id}.pending.json"))
}

#[must_use]
pub fn answer_path(session_home: &Path, question_id: &str) -> PathBuf {
    ask_user_dir(session_home).join(format!("{question_id}.answer.json"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn new_question_id() -> String {
    let ms = now_ms();
    let n = (ms as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    format!("aq_{ms:x}_{n:x}")
}

/// Minimal A2UI-like surface for Admin (claw-ask/v1 catalog). Author: kejiqing
#[must_use]
pub fn build_ask_a2ui(question_id: &str, question: &str, options: Option<&[String]>) -> Value {
    let mut components = vec![json!({
        "id": "title",
        "component": "Text",
        "text": question,
    })];
    if let Some(opts) = options {
        if !opts.is_empty() {
            components.push(json!({
                "id": "options",
                "component": "MultipleChoice",
                "options": opts,
            }));
        }
    }
    components.push(json!({
        "id": "freeText",
        "component": "TextField",
        "label": "回答",
        "placeholder": "输入回答或选择上方选项",
    }));
    components.push(json!({
        "id": "submit",
        "component": "Button",
        "label": "提交",
        "action": "submit",
    }));
    json!({
        "version": "0.8",
        "catalogId": "claw-ask/v1",
        "surfaceId": format!("ask-{question_id}"),
        "components": components,
    })
}

fn resolve_answer_text(raw: &AskUserAnswerFile, options: Option<&[String]>) -> String {
    if let Some(sel) = raw
        .selected
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return sel.to_string();
    }
    let answer = raw.answer.trim();
    if let Some(opts) = options {
        if let Ok(idx) = answer.parse::<usize>() {
            if idx >= 1 && idx <= opts.len() {
                return opts[idx - 1].clone();
            }
        }
    }
    answer.to_string()
}

/// Gateway path: emit ask.user, wait for answer file, return tool_result JSON. Author: kejiqing
pub fn run_ask_user_question_gateway(
    session_home: &Path,
    turn_id: &str,
    input: &AskUserQuestionInput,
    timeout_secs: Option<u64>,
) -> Result<String, String> {
    let question = input.question.trim();
    if question.is_empty() {
        return Err("AskUserQuestion.question must be non-empty".into());
    }
    let question_id = new_question_id();
    let options = input
        .options
        .as_ref()
        .map(|o| {
            o.iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|o| !o.is_empty());
    let a2ui = build_ask_a2ui(&question_id, question, options.as_deref());
    let pending = AskUserPendingFile {
        question_id: question_id.clone(),
        turn_id: turn_id.to_string(),
        question: question.to_string(),
        options: options.clone(),
        a2ui: a2ui.clone(),
        created_at_ms: now_ms(),
    };
    let dir = ask_user_dir(session_home);
    fs::create_dir_all(&dir).map_err(|e| format!("mkdir ask-user: {e}"))?;
    let pending_bytes =
        serde_json::to_vec_pretty(&pending).map_err(|e| format!("serialize pending: {e}"))?;
    fs::write(pending_path(session_home, &question_id), pending_bytes)
        .map_err(|e| format!("write pending: {e}"))?;

    let event = json!({
        "ev": ASK_USER_EVENT,
        "questionId": question_id,
        "turnId": turn_id,
        "question": question,
        "options": options,
        "a2ui": a2ui,
    });
    emit_raw_json(&event).map_err(|e| format!("emit ask.user: {e}"))?;

    let timeout = Duration::from_secs(timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).max(1));
    let poll = Duration::from_millis(DEFAULT_POLL_MS);
    let started = Instant::now();
    let answer_file = answer_path(session_home, &question_id);
    loop {
        if answer_file.is_file() {
            let bytes = fs::read(&answer_file).map_err(|e| format!("read answer: {e}"))?;
            let parsed: AskUserAnswerFile =
                serde_json::from_slice(&bytes).map_err(|e| format!("parse answer: {e}"))?;
            let answer = resolve_answer_text(&parsed, options.as_deref());
            let _ = emit_raw_json(&json!({
                "ev": ASK_USER_CLEARED_EVENT,
                "questionId": question_id,
                "turnId": turn_id,
            }));
            let out = json!({
                "question": question,
                "answer": answer,
                "status": "answered",
                "questionId": question_id,
            });
            return serde_json::to_string_pretty(&out)
                .map_err(|e| format!("serialize tool_result: {e}"));
        }
        if started.elapsed() >= timeout {
            return Err(format!(
                "AskUserQuestion timed out waiting for answer (questionId={question_id})"
            ));
        }
        thread::sleep(poll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_always_enabled() {
        assert!(resolve_ask_user_question_enabled(
            InteractionMode::Plan,
            false
        ));
    }

    #[test]
    fn agent_default_disabled() {
        assert!(!resolve_ask_user_question_enabled(
            InteractionMode::Agent,
            false
        ));
        assert!(resolve_ask_user_question_enabled(
            InteractionMode::Agent,
            true
        ));
    }

    #[test]
    fn gate_strips_and_adds() {
        let mut tools = vec!["Bash".into(), ASK_USER_QUESTION_TOOL_NAME.into()];
        apply_ask_user_tool_gate(&mut tools, false);
        assert!(!tools.iter().any(|t| t == ASK_USER_QUESTION_TOOL_NAME));
        apply_ask_user_tool_gate(&mut tools, true);
        assert!(tools.iter().any(|t| t == ASK_USER_QUESTION_TOOL_NAME));
    }

    #[test]
    fn gate_empty_means_all_untouched() {
        let mut tools = Vec::new();
        apply_ask_user_tool_gate(&mut tools, false);
        assert!(tools.is_empty());
    }

    #[test]
    fn profile_flag() {
        assert!(!ask_user_question_in_agent_from_profile(&json!({})));
        assert!(ask_user_question_in_agent_from_profile(
            &json!({"askUserQuestionInAgent": true})
        ));
    }

    #[test]
    fn a2ui_has_catalog() {
        let v = build_ask_a2ui("aq_1", "选哪个？", Some(&["A".into(), "B".into()]));
        assert_eq!(v["catalogId"], "claw-ask/v1");
        assert_eq!(v["surfaceId"], "ask-aq_1");
    }
}
