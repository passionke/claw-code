//! Outbound LLM headers for observe tap session/turn attribution. Author: kejiqing
//!
//! Workers inject `CLAW_SESSION_ID` / `CLAW_TURN_ID` via e2b exec env. Every LLM
//! client must send the matching HTTP headers so claw-tap can aggregate usage by turn.

use std::collections::BTreeMap;

pub const CLAW_SESSION_HEADER: &str = "claw-session-id";
pub const CLAWCODE_SESSION_HEADER: &str = "clawcode-session-id";
pub const CLAW_TURN_HEADER: &str = "claw-turn-id";

/// Session + turn headers from process env (`CLAW_SESSION_ID`, `CLAW_TURN_ID`).
#[must_use]
pub fn llm_trace_headers_from_env() -> BTreeMap<String, String> {
    let session = std::env::var("CLAW_SESSION_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let turn = std::env::var("CLAW_TURN_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    llm_trace_headers(session.as_deref(), turn.as_deref())
}

/// Build outbound headers. Missing session or turn simply omits that header
/// (tap treats missing turn as "proxy OK, do not record usage").
#[must_use]
pub fn llm_trace_headers(
    session_id: Option<&str>,
    turn_id: Option<&str>,
) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::new();
    if let Some(sid) = session_id.map(str::trim).filter(|s| !s.is_empty()) {
        headers.insert(CLAWCODE_SESSION_HEADER.to_string(), sid.to_string());
        headers.insert(CLAW_SESSION_HEADER.to_string(), sid.to_string());
    }
    if let Some(tid) = turn_id.map(str::trim).filter(|s| !s.is_empty()) {
        headers.insert(CLAW_TURN_HEADER.to_string(), tid.to_string());
    }
    headers
}

/// Explicit session id + turn from `CLAW_TURN_ID` env (solve `DirectApiClient` path).
#[must_use]
pub fn llm_trace_headers_for_session(session_id: &str) -> BTreeMap<String, String> {
    let turn = std::env::var("CLAW_TURN_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    llm_trace_headers(Some(session_id), turn.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn headers_include_session_and_turn() {
        let h = llm_trace_headers(Some("sess-1"), Some("T_abc"));
        assert_eq!(
            h.get(CLAW_SESSION_HEADER).map(String::as_str),
            Some("sess-1")
        );
        assert_eq!(
            h.get(CLAWCODE_SESSION_HEADER).map(String::as_str),
            Some("sess-1")
        );
        assert_eq!(h.get(CLAW_TURN_HEADER).map(String::as_str), Some("T_abc"));
    }

    #[test]
    fn missing_turn_omits_turn_header_only() {
        let h = llm_trace_headers(Some("sess-1"), None);
        assert!(h.contains_key(CLAW_SESSION_HEADER));
        assert!(!h.contains_key(CLAW_TURN_HEADER));
    }

    #[test]
    fn from_env_reads_both() {
        let _guard = env_lock();
        std::env::set_var("CLAW_SESSION_ID", "s-env");
        std::env::set_var("CLAW_TURN_ID", "T_env");
        let h = llm_trace_headers_from_env();
        assert_eq!(
            h.get(CLAW_SESSION_HEADER).map(String::as_str),
            Some("s-env")
        );
        assert_eq!(h.get(CLAW_TURN_HEADER).map(String::as_str), Some("T_env"));
        std::env::remove_var("CLAW_SESSION_ID");
        std::env::remove_var("CLAW_TURN_ID");
    }
}
