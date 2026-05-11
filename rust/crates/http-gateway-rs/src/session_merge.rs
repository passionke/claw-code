//! Effective session id merge and session path validation (continues-by-sid). Author: kejiqing

use std::path::{Path, PathBuf};

use uuid::Uuid;

/// Single directory name under `ds_*/sessions/` aligned with the effective gateway `session_id`
/// when that id is safe as one path segment; otherwise a deterministic 32-hex segment (UUID v5). kejiqing
#[must_use]
pub fn sessions_directory_segment(session_id: &str) -> String {
    let s = session_id.trim();
    if is_safe_sessions_dir_segment(s) {
        s.to_string()
    } else {
        Uuid::new_v5(&Uuid::NAMESPACE_URL, s.as_bytes())
            .simple()
            .to_string()
    }
}

fn is_safe_sessions_dir_segment(s: &str) -> bool {
    if s.is_empty() || s.len() > 200 {
        return false;
    }
    if s == "." || s == ".." {
        return false;
    }
    if s.contains("..") || s.contains('/') || s.contains('\\') {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Whether the inbound request supplied `claw-session-id` / `x-request-id` (vs gateway-generated).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HttpRequestIdKind {
    FromClientHeader,
    Generated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionRoutingError {
    MergeHeaderBodyConflict,
    RegistryRelInvalid,
    AbsNotUnderWorkRoot,
}

impl SessionRoutingError {
    #[must_use]
    pub fn detail(self) -> &'static str {
        match self {
            SessionRoutingError::MergeHeaderBodyConflict => {
                "sessionId conflicts with claw-session-id or x-request-id header"
            }
            SessionRoutingError::RegistryRelInvalid => "invalid session_home in session registry",
            SessionRoutingError::AbsNotUnderWorkRoot => {
                "session workspace path is not under CLAW_WORK_ROOT"
            }
        }
    }
}

#[must_use]
pub fn trim_session_id(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|s| !s.is_empty())
}

/// Body `sessionId` (trimmed) wins over the extension id when present; header/body mismatch is an error.
pub fn merge_effective_session_id(
    body_sid: Option<&str>,
    extension_id: &str,
    kind: HttpRequestIdKind,
) -> Result<String, SessionRoutingError> {
    if let Some(body_sid) = body_sid {
        if matches!(kind, HttpRequestIdKind::FromClientHeader) && body_sid != extension_id {
            return Err(SessionRoutingError::MergeHeaderBodyConflict);
        }
        Ok(body_sid.to_string())
    } else {
        Ok(extension_id.to_string())
    }
}

pub fn validate_session_home_rel(rel: &str) -> Result<(), SessionRoutingError> {
    let rel = rel.trim();
    if rel.is_empty() || rel.contains("..") {
        return Err(SessionRoutingError::RegistryRelInvalid);
    }
    Ok(())
}

pub fn session_home_rel_under_work_root(
    work_root: &Path,
    abs: &Path,
) -> Result<String, SessionRoutingError> {
    let rel = abs
        .strip_prefix(work_root)
        .map_err(|_| SessionRoutingError::AbsNotUnderWorkRoot)?;
    Ok(rel
        .to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_string())
}

#[must_use]
pub fn join_session_home_from_rel(work_root: &Path, rel: &str) -> PathBuf {
    work_root.join(rel.trim().trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct SolveSessionIdBody {
        #[serde(rename = "sessionId")]
        session_id: Option<String>,
        #[serde(rename = "dsId")]
        ds_id: i64,
    }

    #[test]
    fn solve_request_json_session_id_camel_case() {
        let j = r#"{"dsId":1,"userPrompt":"hi","sessionId":"abc-123"}"#;
        let v: SolveSessionIdBody = serde_json::from_str(j).unwrap();
        assert_eq!(v.session_id.as_deref(), Some("abc-123"));
        assert_eq!(v.ds_id, 1);
        let j2 = r#"{"dsId":1,"userPrompt":"hi"}"#;
        let v2: SolveSessionIdBody = serde_json::from_str(j2).unwrap();
        assert!(v2.session_id.is_none());
    }

    #[test]
    fn merge_no_body_uses_extension() {
        assert_eq!(
            merge_effective_session_id(None, "ext-1", HttpRequestIdKind::Generated).unwrap(),
            "ext-1"
        );
    }

    #[test]
    fn merge_body_wins_when_header_generated() {
        assert_eq!(
            merge_effective_session_id(
                Some("sid-body"),
                "generated-uuid",
                HttpRequestIdKind::Generated
            )
            .unwrap(),
            "sid-body"
        );
    }

    #[test]
    fn merge_body_same_as_header_ok() {
        assert_eq!(
            merge_effective_session_id(Some("same"), "same", HttpRequestIdKind::FromClientHeader)
                .unwrap(),
            "same"
        );
    }

    #[test]
    fn merge_header_mismatch_is_error() {
        assert_eq!(
            merge_effective_session_id(Some("body"), "header", HttpRequestIdKind::FromClientHeader),
            Err(SessionRoutingError::MergeHeaderBodyConflict)
        );
    }

    #[test]
    fn trim_session_id_none_empty_whitespace() {
        assert_eq!(trim_session_id(None), None);
        assert_eq!(trim_session_id(Some("")), None);
        assert_eq!(trim_session_id(Some("   ")), None);
        assert_eq!(trim_session_id(Some(" abc ")), Some("abc"));
    }

    #[test]
    fn validate_rel_rejects_dotdot() {
        assert_eq!(
            validate_session_home_rel("ds_1/sessions/../x"),
            Err(SessionRoutingError::RegistryRelInvalid)
        );
        assert!(validate_session_home_rel("ds_1/sessions/u1").is_ok());
    }

    #[test]
    fn session_home_rel_under_work_root_ok() {
        let wr = Path::new("/var/lib/ws");
        let abs = Path::new("/var/lib/ws/ds_1/sessions/u1");
        assert_eq!(
            session_home_rel_under_work_root(wr, abs).unwrap(),
            "ds_1/sessions/u1"
        );
    }

    #[test]
    fn session_home_rel_not_under_root() {
        let wr = Path::new("/var/lib/ws");
        let abs = Path::new("/other/ds_1");
        assert_eq!(
            session_home_rel_under_work_root(wr, abs),
            Err(SessionRoutingError::AbsNotUnderWorkRoot)
        );
    }

    #[test]
    fn join_session_home_from_rel_trims() {
        let wr = Path::new("/w");
        assert_eq!(
            join_session_home_from_rel(wr, "  /ds/s1  "),
            Path::new("/w/ds/s1")
        );
    }

    #[test]
    fn sessions_directory_segment_uses_id_when_safe() {
        assert_eq!(
            sessions_directory_segment("a1b2c3d4e5f678901234567890abCDEF"),
            "a1b2c3d4e5f678901234567890abCDEF"
        );
        assert_eq!(sessions_directory_segment("  my-sid_1.x  "), "my-sid_1.x");
    }

    #[test]
    fn sessions_directory_segment_v5_when_unsafe() {
        let a = sessions_directory_segment("bad/id");
        let b = sessions_directory_segment("bad/id");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, "bad/id");
    }
}
