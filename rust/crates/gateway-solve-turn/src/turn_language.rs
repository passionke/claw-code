//! Per-turn response language inference (step 0) and system-prompt injection. Author: kejiqing

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use api::{InputContentBlock, InputMessage, MessageRequest, ProviderClient};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::gateway_solve_session_persistence_path;
use crate::project_language_pipeline::{render_language_inference_prompt, LanguagePipelineConfig};
use crate::{err, polish_output_from_events, stream_events, GatewaySolveTurnError, HTTP_INTERNAL};

pub const TURN_LANGUAGE_REL: &str = ".claw/turn-language.json";
const LANG_TAG: &str = "[LANG_TAG]";
const FALLBACK_LANGUAGE: &str = "English";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnLanguageFile {
    pub turn_id: String,
    pub language: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub prior_turns_used: usize,
    #[serde(default)]
    pub source: String,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct InferenceLlmJson {
    language: String,
    #[serde(default)]
    reason: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[must_use]
pub fn turn_language_path(session_home: &Path) -> PathBuf {
    session_home.join(TURN_LANGUAGE_REL)
}

/// Locked turn language from step 0 (e.g. `Chinese` / `English` / `Thai`). Author: kejiqing
#[must_use]
pub fn read_turn_language(session_home: &Path) -> Option<String> {
    let raw = fs::read_to_string(turn_language_path(session_home)).ok()?;
    serde_json::from_str::<TurnLanguageFile>(&raw)
        .ok()
        .map(|f| f.language)
        .filter(|s| !s.trim().is_empty())
}

/// Extract prior user message texts from gateway session jsonl (oldest → newest), capped.
#[must_use]
pub fn collect_prior_user_prompts(
    session_home: &Path,
    max_turns: usize,
    max_chars: usize,
) -> (String, usize) {
    let path = gateway_solve_session_persistence_path(session_home);
    let mut texts = Vec::new();
    if path.is_file() {
        if let Ok(contents) = fs::read_to_string(&path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(record) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if record.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                let Some(msg) = record.get("message") else {
                    continue;
                };
                if msg.get("role").and_then(Value::as_str) != Some("user") {
                    continue;
                }
                let Some(blocks) = msg.get("blocks").and_then(Value::as_array) else {
                    continue;
                };
                let mut parts = Vec::new();
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) == Some("text") {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            let t = text.trim();
                            if !t.is_empty() {
                                parts.push(t.to_string());
                            }
                        }
                    }
                }
                if !parts.is_empty() {
                    texts.push(parts.join("\n"));
                }
            }
        }
    }
    let used_count = texts.len();
    if max_turns > 0 && texts.len() > max_turns {
        texts = texts.split_off(texts.len() - max_turns);
    }
    let formatted = format_prior_user_prompts(&texts, max_chars);
    (formatted, used_count.min(max_turns))
}

/// Like [`collect_prior_user_prompts`], but drops a trailing user text equal to `exclude` (current turn).
#[must_use]
pub fn collect_prior_user_prompts_excluding(
    session_home: &Path,
    max_turns: usize,
    max_chars: usize,
    exclude: &str,
) -> (String, usize) {
    let path = gateway_solve_session_persistence_path(session_home);
    let mut texts = Vec::new();
    if path.is_file() {
        if let Ok(contents) = fs::read_to_string(&path) {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(record) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if record.get("type").and_then(Value::as_str) != Some("message") {
                    continue;
                }
                let Some(msg) = record.get("message") else {
                    continue;
                };
                if msg.get("role").and_then(Value::as_str) != Some("user") {
                    continue;
                }
                let Some(blocks) = msg.get("blocks").and_then(Value::as_array) else {
                    continue;
                };
                let mut parts = Vec::new();
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) == Some("text") {
                        if let Some(text) = block.get("text").and_then(Value::as_str) {
                            let t = text.trim();
                            if !t.is_empty() {
                                parts.push(t.to_string());
                            }
                        }
                    }
                }
                if !parts.is_empty() {
                    texts.push(parts.join("\n"));
                }
            }
        }
    }
    let exclude_trim = exclude.trim();
    if let Some(last) = texts.last() {
        if last.trim() == exclude_trim {
            texts.pop();
        }
    }
    let used_count = texts.len();
    if max_turns > 0 && texts.len() > max_turns {
        texts = texts.split_off(texts.len() - max_turns);
    }
    let formatted = format_prior_user_prompts(&texts, max_chars);
    (formatted, used_count.min(max_turns))
}

fn format_prior_user_prompts(texts: &[String], max_chars: usize) -> String {
    if texts.is_empty() {
        return String::from("(none)");
    }
    let mut selected: Vec<&str> = texts.iter().map(String::as_str).collect();
    loop {
        let body: Vec<String> = selected
            .iter()
            .enumerate()
            .map(|(i, t)| format!("[Prior turn {}] {t}", i + 1))
            .collect();
        let joined = body.join("\n");
        if joined.chars().count() <= max_chars || selected.len() <= 1 {
            return joined;
        }
        selected.remove(0);
    }
}

#[must_use]
pub fn apply_lang_tag_to_text(text: &str, language: &str) -> String {
    text.replace(LANG_TAG, language)
}

#[must_use]
pub fn response_language_system_section(language: &str) -> String {
    format!(
        "# Response language (locked for this turn)\n\
         All user-visible text in this turn — including report_progress and the final assistant reply — MUST use: {language}.\n\
         Respect explicit user instructions (e.g. \"answer in English\") over the language of the question text.\n\
         System prompt and skill text in other languages are instructional metadata only."
    )
}

/// Replace `[LANG_TAG]` in each section and append the locked-language block.
pub fn inject_language_into_system_prompt(system_prompt: &mut Vec<String>, language: &str) {
    for section in system_prompt.iter_mut() {
        *section = apply_lang_tag_to_text(section, language);
    }
    system_prompt.push(response_language_system_section(language));
}

pub fn persist_turn_language(session_home: &Path, file: &TurnLanguageFile) -> Result<(), String> {
    let path = turn_language_path(session_home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create turn-language dir failed: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(file)
        .map_err(|e| format!("serialize turn-language failed: {e}"))?;
    fs::write(&path, bytes).map_err(|e| format!("write turn-language failed: {e}"))
}

/// Unwrap `polish_output_from_events` envelope (`{"message":"…"}`) when present.
fn inference_llm_text_payload(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if let Some(msg) = v.get("message").and_then(Value::as_str) {
            return msg.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn parse_inference_json(text: &str) -> Result<InferenceLlmJson, String> {
    let trimmed = inference_llm_text_payload(text);
    if let Ok(v) = serde_json::from_str::<InferenceLlmJson>(&trimmed) {
        return Ok(v);
    }
    let start = trimmed
        .find('{')
        .ok_or_else(|| "no JSON object in inference output".to_string())?;
    let end = trimmed
        .rfind('}')
        .ok_or_else(|| "no JSON object in inference output".to_string())?;
    serde_json::from_str(&trimmed[start..=end])
        .map_err(|e| format!("parse inference JSON failed: {e}"))
}

async fn run_language_inference_llm(
    user_message: &str,
    model: &str,
    session_id: &str,
) -> Result<String, GatewaySolveTurnError> {
    let provider = ProviderClient::from_model(model).map_err(|e| {
        err(
            HTTP_INTERNAL,
            format!("language inference provider init: {e}"),
        )
    })?;
    let req = MessageRequest {
        model: model.to_string(),
        max_tokens: 256,
        messages: vec![InputMessage {
            role: "user".to_string(),
            content: vec![InputContentBlock::Text {
                text: user_message.to_string(),
            }],
        }],
        system: Some(
            "You classify the output language for a business assistant turn. Respond with JSON only."
                .to_string(),
        ),
        tools: None,
        tool_choice: None,
        stream: true,
        thinking_enabled: Some(false),
        extra_headers: BTreeMap::from([
            (
                "clawcode-session-id".to_string(),
                session_id.to_string(),
            ),
            ("claw-session-id".to_string(), session_id.to_string()),
        ]),
        ..Default::default()
    };
    let mut noop_delta = |_delta: &str| {};
    let events = stream_events(&provider, &req, Some(&mut noop_delta))
        .await
        .map_err(|e| {
            err(
                HTTP_INTERNAL,
                format!("language inference stream failed: {e}"),
            )
        })?;
    let (text, _) = polish_output_from_events(&events, model)?;
    Ok(text)
}

/// Infer language string from a rendered inference prompt (no persist).
pub async fn infer_turn_language_only(
    user_message: &str,
    model: &str,
    session_id: &str,
) -> Result<String, GatewaySolveTurnError> {
    match run_language_inference_llm(user_message, model, session_id).await {
        Ok(raw) => match parse_inference_json(&raw) {
            Ok(parsed) => {
                let lang = parsed.language.trim().to_string();
                if lang.is_empty() {
                    Ok(FALLBACK_LANGUAGE.to_string())
                } else {
                    Ok(lang)
                }
            }
            Err(_) => Ok(FALLBACK_LANGUAGE.to_string()),
        },
        Err(_) => Ok(FALLBACK_LANGUAGE.to_string()),
    }
}

/// Step 0: infer language, persist, return locked language string.
pub async fn infer_and_persist_turn_language(
    session_home: &Path,
    current_user_prompt: &str,
    turn_id: &str,
    session_id: &str,
    model: &str,
    pipeline: &LanguagePipelineConfig,
) -> Result<String, GatewaySolveTurnError> {
    let (prior_block, prior_turns_used) = collect_prior_user_prompts(
        session_home,
        pipeline.language_inference_prior_turns,
        pipeline.language_inference_prior_max_chars,
    );
    let user_message = render_language_inference_prompt(
        &pipeline.language_inference_prompt,
        &prior_block,
        current_user_prompt.trim(),
    );
    let (language, reason, source) =
        match run_language_inference_llm(&user_message, model, session_id).await {
            Ok(raw) => match parse_inference_json(&raw) {
                Ok(parsed) => {
                    let lang = parsed.language.trim().to_string();
                    if lang.is_empty() {
                        (
                            FALLBACK_LANGUAGE.to_string(),
                            Some(format!("empty language in LLM output; raw={raw}")),
                            "fallback".to_string(),
                        )
                    } else {
                        (lang, parsed.reason, "inference".to_string())
                    }
                }
                Err(e) => (
                    FALLBACK_LANGUAGE.to_string(),
                    Some(format!("{e}; raw={raw}")),
                    "fallback".to_string(),
                ),
            },
            Err(e) => (
                FALLBACK_LANGUAGE.to_string(),
                Some(e.message),
                "fallback".to_string(),
            ),
        };
    let file = TurnLanguageFile {
        turn_id: turn_id.to_string(),
        language: language.clone(),
        reason,
        prior_turns_used,
        source,
        updated_at_ms: now_ms(),
    };
    persist_turn_language(session_home, &file)
        .map_err(|e| err(HTTP_INTERNAL, format!("persist turn-language: {e}")))?;
    Ok(language)
}

/// Sync wrapper for `run_gateway_solve_turn`.
pub fn infer_and_persist_turn_language_blocking(
    session_home: &Path,
    current_user_prompt: &str,
    turn_id: &str,
    session_id: &str,
    model: &str,
    pipeline: &LanguagePipelineConfig,
) -> Result<String, GatewaySolveTurnError> {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(infer_and_persist_turn_language(
            session_home,
            current_user_prompt,
            turn_id,
            session_id,
            model,
            pipeline,
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn collect_prior_formats_and_caps() {
        let dir = tempfile::tempdir().unwrap();
        let path = gateway_solve_session_persistence_path(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let line = |role: &str, text: &str| {
            serde_json::json!({
                "type": "message",
                "message": {
                    "role": role,
                    "blocks": [{"type": "text", "text": text}]
                }
            })
            .to_string()
        };
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "{}", line("user", "first")).unwrap();
        writeln!(f, "{}", line("assistant", "ok")).unwrap();
        writeln!(f, "{}", line("user", "second")).unwrap();
        let (block, count) = collect_prior_user_prompts(dir.path(), 5, 3000);
        assert_eq!(count, 2);
        assert!(block.contains("[Prior turn 1] first"));
        assert!(block.contains("[Prior turn 2] second"));
    }

    #[test]
    fn collect_prior_none_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (block, count) = collect_prior_user_prompts(dir.path(), 5, 3000);
        assert_eq!(count, 0);
        assert_eq!(block, "(none)");
    }

    #[test]
    fn inject_replaces_lang_tag_and_appends_section() {
        let mut sections = vec![format!("Use {LANG_TAG} only.")];
        inject_language_into_system_prompt(&mut sections, "Thai");
        assert!(!sections[0].contains(LANG_TAG));
        assert!(sections[0].contains("Thai"));
        assert!(sections[1].contains("# Response language"));
        assert!(sections[1].contains("Thai"));
    }

    #[test]
    fn parse_inference_json_from_prose_wrapper() {
        let raw = r#"Sure. {"language":"English","reason":"user asked"}"#;
        let p = parse_inference_json(raw).unwrap();
        assert_eq!(p.language, "English");
    }

    #[test]
    fn parse_inference_json_from_polish_envelope() {
        let raw = r#"{"iterations":1,"message":"{\"language\":\"Chinese\",\"reason\":\"The current turn user message is in Chinese.\"}","model":"deepseek-v4-pro","usage":{"input_tokens":1,"output_tokens":1}}"#;
        let p = parse_inference_json(raw).unwrap();
        assert_eq!(p.language, "Chinese");
    }
}
