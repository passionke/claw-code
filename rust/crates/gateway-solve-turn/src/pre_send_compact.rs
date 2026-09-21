//! Compact gateway-solve jsonl before the first LLM call when a window is set. Author: kejiqing

use std::path::Path;

use api::ToolDefinition;
use runtime::{compact_session, estimate_session_prompt_units, CompactionConfig, Session};

use crate::{err, GatewaySolveTurnError, HTTP_BAD_REQUEST};

pub const CONTEXT_WINDOW_ENV: &str = "CLAW_CONTEXT_WINDOW_TOKENS";

#[must_use]
pub fn context_window_tokens_from_env() -> Option<u32> {
    std::env::var(CONTEXT_WINDOW_ENV)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n: &u32| *n > 0)
}

fn tool_prompt_units(tool: &ToolDefinition) -> usize {
    tool.name.len()
        + tool.description.as_deref().map_or(0, str::len)
        + tool.input_schema.to_string().len()
}

#[must_use]
pub fn estimate_solve_prompt_units(
    session: &Session,
    system_prompt: &[String],
    tools: &[ToolDefinition],
) -> usize {
    estimate_session_prompt_units(session)
        + system_prompt.iter().map(String::len).sum::<usize>()
        + tools.iter().map(tool_prompt_units).sum::<usize>()
}

/// When env window is unset, return the session unchanged (same as today).
pub fn maybe_compact_before_llm(
    session: Session,
    jsonl_path: &Path,
    system_prompt: &[String],
    tools: &[ToolDefinition],
) -> Result<Session, GatewaySolveTurnError> {
    let Some(window) = context_window_tokens_from_env() else {
        return Ok(session);
    };
    compact_session_to_window(session, jsonl_path, window, system_prompt, tools)
}

pub fn compact_session_to_window(
    session: Session,
    jsonl_path: &Path,
    window: u32,
    system_prompt: &[String],
    tools: &[ToolDefinition],
) -> Result<Session, GatewaySolveTurnError> {
    let window_usize = usize::try_from(window).unwrap_or(usize::MAX);
    let before = estimate_solve_prompt_units(&session, system_prompt, tools);
    if before <= window_usize {
        return Ok(session);
    }
    let result = compact_session(
        &session,
        CompactionConfig {
            preserve_recent_messages: 4,
            max_estimated_tokens: 0,
        },
    );
    let compacted = result.compacted_session;
    if result.removed_message_count > 0 {
        compacted.save_to_path(jsonl_path).map_err(|e| {
            err(
                HTTP_BAD_REQUEST,
                format!("rewrite compacted session jsonl failed: {e}"),
            )
        })?;
    }
    let after = estimate_solve_prompt_units(&compacted, system_prompt, tools);
    if after > window_usize {
        return Err(err(
            HTTP_BAD_REQUEST,
            format!(
                "session exceeds model context window {window}: estimated {after} prompt units after compact (before compact {before}); shorten history or set a larger Admin contextWindowTokens"
            ),
        ));
    }
    Ok(compacted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime::{ContentBlock, ConversationMessage, MessageRole, Session};

    fn zh_block(n: usize) -> String {
        "中文日志块".repeat(n)
    }

    fn long_history() -> Session {
        let mut session = Session::new();
        let dump = zh_block(8_000);
        session.messages = vec![
            ConversationMessage::user_text(dump.clone()),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: dump.clone() }]),
            ConversationMessage::user_text(dump.clone()),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: dump }]),
            ConversationMessage::user_text("recent-1"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "recent-2".into(),
            }]),
            ConversationMessage::user_text("recent-3"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "recent-4".into(),
            }]),
        ];
        session
    }

    #[test]
    fn chinese_log_block_is_not_divided_by_four() {
        let text = zh_block(1);
        assert!(text.len() > text.chars().count());
        let mut session = Session::new();
        session.messages = vec![ConversationMessage::user_text(text.clone())];
        assert_eq!(estimate_session_prompt_units(&session), text.len());
        assert!(estimate_session_prompt_units(&session) > text.len() / 4);
    }

    #[test]
    fn no_window_path_is_identity_via_unset_env() {
        let _lock = env_lock();
        std::env::remove_var(CONTEXT_WINDOW_ENV);
        let session = long_history();
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gateway-solve-session.jsonl");
        let out = maybe_compact_before_llm(session.clone(), &path, &[], &[]).unwrap();
        assert_eq!(out.messages.len(), session.messages.len());
        assert!(!path.exists());
    }

    #[test]
    fn over_window_compacts_and_rewrites_jsonl() {
        let session = long_history();
        let before = estimate_solve_prompt_units(&session, &[], &[]);
        assert!(before > 2_000, "fixture must exceed a small window");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gateway-solve-session.jsonl");
        let out = compact_session_to_window(session, &path, 50_000, &[], &[]).unwrap();
        assert!(path.is_file());
        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(disk.len() < before);
        assert!(estimate_solve_prompt_units(&out, &[], &[]) <= 50_000);
        assert_eq!(out.messages[0].role, MessageRole::System);
    }

    #[test]
    fn still_over_window_after_compact_fails_locally() {
        let mut session = Session::new();
        let dump = zh_block(40_000);
        session.messages = vec![
            ConversationMessage::user_text(dump.clone()),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: dump.clone() }]),
            ConversationMessage::user_text(dump.clone()),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: dump }]),
        ];
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gateway-solve-session.jsonl");
        let err = compact_session_to_window(session, &path, 100, &[], &[]).unwrap_err();
        assert_eq!(err.status, HTTP_BAD_REQUEST);
        assert!(err.message.contains("100"));
        assert!(err.message.contains("context window"));
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
