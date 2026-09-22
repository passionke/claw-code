//! One Admin context window drives spill and pre-stream compact. Author: kejiqing
//!
//! `CLAW_CONTEXT_WINDOW_TOKENS` is the only size. Compact starts at
//! `window * CLAW_CONTEXT_COMPACT_RATIO_PERCENT / 100` (default 80).
//! A single tool result enters the session in full only while it stays under
//! `window * TOOL_RESULT_INLINE_WINDOW_PERCENT / 100`. Larger results are
//! written beside the session jsonl; the `tool_result` keeps a path and a
//! short head/tail.

use std::fs;
use std::path::{Path, PathBuf};

use crate::compact::{compact_session, estimate_session_prompt_units, CompactionConfig};
use crate::session::Session;

pub const CONTEXT_WINDOW_ENV: &str = "CLAW_CONTEXT_WINDOW_TOKENS";
pub const COMPACT_RATIO_ENV: &str = "CLAW_CONTEXT_COMPACT_RATIO_PERCENT";
pub const DEFAULT_COMPACT_RATIO_PERCENT: u32 = 80;
/// Hardcoded slice of the window. Not a third user setting. Author: kejiqing
pub const TOOL_RESULT_INLINE_WINDOW_PERCENT: u32 = 5;

const SPILL_EDGE_BYTES: usize = 1_024;
const PRESERVE_RECENT_MESSAGES: usize = 4;

#[must_use]
pub fn context_window_tokens_from_env() -> Option<u32> {
    std::env::var(CONTEXT_WINDOW_ENV)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|n: &u32| *n > 0)
}

#[must_use]
pub fn normalize_compact_ratio_percent(raw: Option<u32>) -> u32 {
    match raw {
        Some(n) if (1..=100).contains(&n) => n,
        _ => DEFAULT_COMPACT_RATIO_PERCENT,
    }
}

#[must_use]
pub fn compact_ratio_percent_from_env() -> u32 {
    let raw = std::env::var(COMPACT_RATIO_ENV)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    normalize_compact_ratio_percent(raw)
}

#[must_use]
pub fn compact_trigger_units(window: u32, ratio_percent: u32) -> usize {
    let ratio = u64::from(normalize_compact_ratio_percent(Some(ratio_percent)));
    let trigger = u64::from(window).saturating_mul(ratio) / 100;
    usize::try_from(trigger).unwrap_or(usize::MAX)
}

#[must_use]
pub fn tool_result_inline_limit(window: u32) -> usize {
    let limit =
        u64::from(window).saturating_mul(u64::from(TOOL_RESULT_INLINE_WINDOW_PERCENT)) / 100;
    usize::try_from(limit).unwrap_or(usize::MAX).max(1)
}

#[must_use]
pub fn prompt_units(session: &Session, system_prompt: &[String], auxiliary_units: usize) -> usize {
    estimate_session_prompt_units(session)
        + system_prompt.iter().map(String::len).sum::<usize>()
        + auxiliary_units
}

/// Compact when the prompt is over `window * ratio`. Still over the window
/// after compact is a local error: do not send that request. Author: kejiqing
pub fn compact_session_for_stream(
    session: Session,
    system_prompt: &[String],
    auxiliary_units: usize,
) -> Result<Session, Box<(Session, String)>> {
    let Some(window) = context_window_tokens_from_env() else {
        return Ok(session);
    };
    let trigger = compact_trigger_units(window, compact_ratio_percent_from_env());
    let window_usize = usize::try_from(window).unwrap_or(usize::MAX);
    let before = prompt_units(&session, system_prompt, auxiliary_units);
    if before <= trigger {
        return Ok(session);
    }
    let result = compact_session(
        &session,
        CompactionConfig {
            preserve_recent_messages: PRESERVE_RECENT_MESSAGES,
            max_estimated_tokens: 0,
        },
    );
    let compacted = result.compacted_session;
    if result.removed_message_count > 0 {
        if let Some(path) = compacted.persistence_path().map(Path::to_path_buf) {
            if let Err(e) = compacted.save_to_path(&path) {
                return Err(Box::new((
                    compacted,
                    format!("rewrite compacted session jsonl failed: {e}"),
                )));
            }
        }
    }
    let after = prompt_units(&compacted, system_prompt, auxiliary_units);
    if after > window_usize {
        return Err(Box::new((
            compacted,
            format!(
                "session exceeds model context window {window}: estimated {after} prompt units after compact (before compact {before}); shorten history or set a larger Admin contextWindowTokens"
            ),
        )));
    }
    Ok(compacted)
}

/// Full tool output stays in the session only while it fits the inline cap.
/// No window configured: return the output unchanged. Author: kejiqing
#[must_use]
pub fn spill_tool_output_for_context(
    session_jsonl: Option<&Path>,
    tool_use_id: &str,
    tool_name: &str,
    output: String,
) -> String {
    let Some(window) = context_window_tokens_from_env() else {
        return output;
    };
    let limit = tool_result_inline_limit(window);
    if output.len() <= limit {
        return output;
    }
    let Some(jsonl) = session_jsonl else {
        return bounded_stub_without_file(&output, tool_name, limit);
    };
    let Some(dir) = jsonl.parent() else {
        return bounded_stub_without_file(&output, tool_name, limit);
    };
    let path = tool_output_path(dir, tool_use_id);
    if let Err(error) = write_tool_output(&path, &output) {
        return fit_to_limit(
            &format!(
                "Tool output ({bytes} bytes) could not be written to disk ({error}). It was omitted from context.",
                bytes = output.len()
            ),
            limit,
        );
    }
    let stub = spill_stub(&path, tool_name, &output);
    fit_to_limit(&stub, limit)
}

fn tool_output_path(session_dir: &Path, tool_use_id: &str) -> PathBuf {
    session_dir
        .join("tool-outputs")
        .join(format!("{}.txt", sanitize_tool_use_id(tool_use_id)))
}

fn sanitize_tool_use_id(tool_use_id: &str) -> String {
    let cleaned: String = tool_use_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "tool-output".to_string()
    } else {
        cleaned
    }
}

fn write_tool_output(path: &Path, output: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, output).map_err(|e| e.to_string())
}

fn spill_stub(path: &Path, tool_name: &str, output: &str) -> String {
    let lines = output.lines().count();
    format!(
        "Path: {path}\nTool `{tool_name}` output spilled to disk ({bytes} bytes, {lines} lines).\nUse read or grep on this path. Do not paste the full output back into the conversation.\n\n--- head ---\n{head}\n--- tail ---\n{tail}",
        bytes = output.len(),
        path = path.display(),
        head = prefix_bytes(output, SPILL_EDGE_BYTES),
        tail = suffix_bytes(output, SPILL_EDGE_BYTES),
    )
}

fn bounded_stub_without_file(output: &str, tool_name: &str, limit: usize) -> String {
    let _ = tool_name;
    fit_to_limit(
        &format!(
            "Tool output omitted from context ({bytes} bytes). No session file was available to store it.\n\n--- head ---\n{head}",
            bytes = output.len(),
            head = prefix_bytes(output, SPILL_EDGE_BYTES),
        ),
        limit,
    )
}

fn fit_to_limit(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    prefix_bytes(text, limit).to_string()
}

fn prefix_bytes(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn suffix_bytes(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len().saturating_sub(max);
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Serializes tests that read or write the context-window env vars. Author: kejiqing
#[cfg(test)]
pub(crate) fn context_budget_test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ContentBlock, ConversationMessage, MessageRole, Session};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        super::context_budget_test_env_lock()
    }

    fn zh_block(n: usize) -> String {
        "中文日志块".repeat(n)
    }

    #[test]
    fn ratio_defaults_and_rejects_out_of_range() {
        assert_eq!(normalize_compact_ratio_percent(None), 80);
        assert_eq!(normalize_compact_ratio_percent(Some(0)), 80);
        assert_eq!(normalize_compact_ratio_percent(Some(101)), 80);
        assert_eq!(normalize_compact_ratio_percent(Some(60)), 60);
        assert_eq!(compact_trigger_units(1000, 80), 800);
        assert_eq!(tool_result_inline_limit(1000), 50);
    }

    #[test]
    fn spill_writes_file_and_keeps_path_in_context() {
        let _lock = env_lock();
        std::env::set_var(CONTEXT_WINDOW_ENV, "60000");
        let tmp = tempfile::tempdir().unwrap();
        let jsonl = tmp.path().join("gateway-solve-session.jsonl");
        let output = "x".repeat(10_000);
        let shown = spill_tool_output_for_context(Some(&jsonl), "call-1", "mcp__sls", output);
        assert!(shown.contains("tool-outputs/call-1.txt"), "{shown}");
        assert!(shown.len() <= tool_result_inline_limit(60_000));
        let disk = fs::read_to_string(tmp.path().join("tool-outputs/call-1.txt")).unwrap();
        assert_eq!(disk.len(), 10_000);
        std::env::remove_var(CONTEXT_WINDOW_ENV);
    }

    #[test]
    fn small_output_stays_inline() {
        let _lock = env_lock();
        std::env::set_var(CONTEXT_WINDOW_ENV, "10000");
        let shown = spill_tool_output_for_context(None, "id", "read", "short".into());
        assert_eq!(shown, "short");
        std::env::remove_var(CONTEXT_WINDOW_ENV);
    }

    #[test]
    fn unset_window_does_not_spill() {
        let _lock = env_lock();
        std::env::remove_var(CONTEXT_WINDOW_ENV);
        let output = "y".repeat(50_000);
        let shown = spill_tool_output_for_context(None, "id", "read", output.clone());
        assert_eq!(shown, output);
    }

    #[test]
    fn over_trigger_compacts_and_rewrites_jsonl() {
        let _lock = env_lock();
        std::env::set_var(CONTEXT_WINDOW_ENV, "80000");
        std::env::set_var(COMPACT_RATIO_ENV, "50");
        let mut session = Session::new();
        let dump = zh_block(2_000);
        session.messages = vec![
            ConversationMessage::user_text(dump.clone()),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: dump.clone() }]),
            ConversationMessage::user_text(dump.clone()),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: dump }]),
            ConversationMessage::user_text("recent-1"),
            ConversationMessage::assistant(vec![ContentBlock::Text {
                text: "recent-2".into(),
            }]),
        ];
        let before = prompt_units(&session, &[], 0);
        assert!(before > compact_trigger_units(80_000, 50));
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gateway-solve-session.jsonl");
        session = session.with_persistence_path(path.clone());
        let out = compact_session_for_stream(session, &[], 0).unwrap();
        assert!(path.is_file());
        assert!(prompt_units(&out, &[], 0) <= 80_000);
        assert_eq!(out.messages[0].role, MessageRole::System);
        std::env::remove_var(CONTEXT_WINDOW_ENV);
        std::env::remove_var(COMPACT_RATIO_ENV);
    }

    #[test]
    fn still_over_window_after_compact_fails() {
        let _lock = env_lock();
        std::env::set_var(CONTEXT_WINDOW_ENV, "100");
        std::env::set_var(COMPACT_RATIO_ENV, "80");
        let mut session = Session::new();
        let dump = zh_block(40);
        session.messages = vec![
            ConversationMessage::user_text(dump.clone()),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: dump.clone() }]),
            ConversationMessage::user_text(dump.clone()),
            ConversationMessage::assistant(vec![ContentBlock::Text { text: dump }]),
        ];
        let err = compact_session_for_stream(session, &[], 0).unwrap_err();
        assert!(err.1.contains("100"));
        assert!(err.1.contains("context window"));
        assert!(
            !err.0.messages.is_empty(),
            "failed compact must keep the session"
        );
        std::env::remove_var(CONTEXT_WINDOW_ENV);
        std::env::remove_var(COMPACT_RATIO_ENV);
    }

    #[test]
    fn below_trigger_does_not_rewrite_jsonl() {
        let _lock = env_lock();
        std::env::set_var(CONTEXT_WINDOW_ENV, "100000");
        std::env::set_var(COMPACT_RATIO_ENV, "80");
        let mut session = Session::new();
        session.messages = vec![ConversationMessage::user_text("short")];
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gateway-solve-session.jsonl");
        session = session.with_persistence_path(path.clone());
        let out = compact_session_for_stream(session, &[], 0).unwrap();
        assert!(!path.exists());
        assert_eq!(out.messages.len(), 1);
        std::env::remove_var(CONTEXT_WINDOW_ENV);
        std::env::remove_var(COMPACT_RATIO_ENV);
    }

    #[test]
    fn system_prompt_counts_toward_the_window() {
        let _lock = env_lock();
        std::env::set_var(CONTEXT_WINDOW_ENV, "1000");
        std::env::set_var(COMPACT_RATIO_ENV, "80");
        let mut session = Session::new();
        session.messages = vec![ConversationMessage::user_text("hi")];
        let err = compact_session_for_stream(session, &[zh_block(200)], 0).unwrap_err();
        assert!(err.1.contains("context window"));
        assert_eq!(err.0.messages.len(), 1);
        std::env::remove_var(CONTEXT_WINDOW_ENV);
        std::env::remove_var(COMPACT_RATIO_ENV);
    }

    #[test]
    fn reloaded_jsonl_keeps_summary_and_drops_early_body() {
        let _lock = env_lock();
        std::env::set_var(CONTEXT_WINDOW_ENV, "80000");
        std::env::set_var(COMPACT_RATIO_ENV, "50");
        let marker = "ENDMARKER_ONLY_IN_DROPPED";
        let mut session = Session::new();
        let dump = format!("{}{}", zh_block(2_000), marker);
        session.messages = vec![
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
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gateway-solve-session.jsonl");
        session = session.with_persistence_path(path.clone());
        let out = compact_session_for_stream(session, &[], 0).unwrap();
        assert_eq!(out.messages[0].role, MessageRole::System);
        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(disk.contains("Summary"));
        assert!(disk.contains("recent-1"));
        assert!(
            !disk.contains(marker),
            "dropped tool/user body must not survive in the next-turn jsonl"
        );
        let reloaded = Session::load_from_path(&path).unwrap();
        assert_eq!(reloaded.messages[0].role, MessageRole::System);
        std::env::remove_var(CONTEXT_WINDOW_ENV);
        std::env::remove_var(COMPACT_RATIO_ENV);
    }

    #[test]
    fn spill_sanitizes_tool_id_and_keeps_utf8_head() {
        let _lock = env_lock();
        std::env::set_var(CONTEXT_WINDOW_ENV, "60000");
        let tmp = tempfile::tempdir().unwrap();
        let jsonl = tmp.path().join("gateway-solve-session.jsonl");
        let output = format!("{}{}", "中".repeat(4_000), "TAIL_MARKER");
        let shown = spill_tool_output_for_context(
            Some(&jsonl),
            "call/../../evil",
            "mcp__sls-prod__sls_execute_sql",
            output.clone(),
        );
        assert!(shown.contains("mcp__sls-prod__sls_execute_sql"));
        assert!(
            shown.contains("tool-outputs/call_______evil.txt"),
            "{shown}"
        );
        assert!(!shown.contains(output.as_str()));
        assert!(shown.is_char_boundary(shown.len()));
        let disk = fs::read_to_string(tmp.path().join("tool-outputs/call_______evil.txt")).unwrap();
        assert!(disk.contains("TAIL_MARKER"));
        assert_eq!(disk, output);
        std::env::remove_var(CONTEXT_WINDOW_ENV);
    }

    #[test]
    fn spill_without_session_file_omits_the_body() {
        let _lock = env_lock();
        std::env::set_var(CONTEXT_WINDOW_ENV, "60000");
        let output = "中".repeat(8_000);
        let shown = spill_tool_output_for_context(None, "id", "bash", output.clone());
        assert!(shown.len() <= tool_result_inline_limit(60_000));
        assert!(!shown.contains(&output));
        assert!(shown.contains("omitted from context"));
        std::env::remove_var(CONTEXT_WINDOW_ENV);
    }
}
