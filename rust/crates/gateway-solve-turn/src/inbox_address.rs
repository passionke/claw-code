//! Mailbox address: `{sessionId}@{projId}.{clusterId}`. Author: kejiqing

use serde::{Deserialize, Serialize};

/// Parsed mailbox address. Author: kejiqing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxAddress {
    pub session_id: String,
    pub proj_id: i64,
    pub cluster_id: String,
}

impl MailboxAddress {
    /// Format canonical address. Author: kejiqing
    #[must_use]
    pub fn format(&self) -> String {
        format!(
            "{}@{}.{}",
            self.session_id.trim(),
            self.proj_id,
            self.cluster_id.trim()
        )
    }

    /// Build from parts; session_id and cluster_id must be non-empty. Author: kejiqing
    pub fn new(session_id: impl Into<String>, proj_id: i64, cluster_id: impl Into<String>) -> Result<Self, String> {
        let session_id = session_id.into().trim().to_string();
        let cluster_id = cluster_id.into().trim().to_string();
        if session_id.is_empty() {
            return Err("mailbox address requires sessionId".into());
        }
        if session_id.contains('@') || session_id.contains('.') {
            return Err("sessionId must not contain '@' or '.'".into());
        }
        if proj_id < 1 {
            return Err("projId must be >= 1".into());
        }
        if cluster_id.is_empty() {
            return Err("mailbox address requires clusterId".into());
        }
        if cluster_id.contains('@') {
            return Err("clusterId must not contain '@'".into());
        }
        Ok(Self {
            session_id,
            proj_id,
            cluster_id,
        })
    }
}

/// Parse `sessionId@projId.clusterId`. Author: kejiqing
pub fn parse_mailbox_address(raw: &str) -> Result<MailboxAddress, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("empty mailbox address".into());
    }
    let (session_id, rest) = raw
        .split_once('@')
        .ok_or_else(|| format!("invalid mailbox address {raw:?}; expected sessionId@projId.clusterId"))?;
    if session_id.is_empty() {
        return Err("mailbox address missing sessionId before '@'".into());
    }
    if session_id.contains('.') {
        return Err("sessionId must not contain '.'".into());
    }
    let (proj_raw, cluster_id) = rest
        .split_once('.')
        .ok_or_else(|| format!("invalid mailbox address {raw:?}; expected sessionId@projId.clusterId"))?;
    if proj_raw.is_empty() || cluster_id.is_empty() {
        return Err(format!(
            "invalid mailbox address {raw:?}; projId and clusterId required"
        ));
    }
    // cluster may contain dots (e.g. pre-claw-01 is fine; if more dots, take first as proj, rest as cluster)
    // Spec: projId.clusterId — clusterId itself should not need dots; if rest has more dots, reject for KISS.
    if cluster_id.contains('.') {
        return Err(format!(
            "invalid mailbox address {raw:?}; clusterId must not contain '.'"
        ));
    }
    let proj_id: i64 = proj_raw
        .parse()
        .map_err(|_| format!("invalid projId in mailbox address {raw:?}"))?;
    MailboxAddress::new(session_id, proj_id, cluster_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let a = MailboxAddress::new("sess_abc", 42, "pre-claw-01").unwrap();
        assert_eq!(a.format(), "sess_abc@42.pre-claw-01");
        assert_eq!(parse_mailbox_address(&a.format()).unwrap(), a);
    }

    #[test]
    fn rejects_missing_session() {
        assert!(parse_mailbox_address("@42.cluster").is_err());
        assert!(parse_mailbox_address("42.cluster").is_err());
    }

    #[test]
    fn rejects_bad_proj() {
        assert!(parse_mailbox_address("s@x.cluster").is_err());
        assert!(parse_mailbox_address("s@0.cluster").is_err());
    }
}
