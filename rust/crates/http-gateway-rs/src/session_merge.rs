//! Effective session id merge and session path validation (continues-by-sid). Author: kejiqing

use std::path::{Path, PathBuf};

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
}
