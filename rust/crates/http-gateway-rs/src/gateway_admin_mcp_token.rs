//! Admin MCP bearer tokens (global settings). Author: kejiqing
//!
//! Tokens gate `POST /v1/admin/mcp` only; Admin UI session auth is unchanged.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::gateway_global_settings::{
    get_gateway_global_settings, save_gateway_global_settings, GatewayGlobalSettingsStore,
};
use crate::session_db::GatewaySessionDb;

pub const ADMIN_MCP_HTTP_PATH: &str = "/v1/admin/mcp";
pub const TOKEN_PREFIX: &str = "camt_";
const TEMPORARY_TTL_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AdminMcpTokenKind {
    Temporary,
    Permanent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminMcpTokenEntry {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub kind: AdminMcpTokenKind,
    #[serde(rename = "tokenHash")]
    pub token_hash: String,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "expiresAtMs", skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    #[serde(rename = "revokedAtMs", skip_serializing_if = "Option::is_none")]
    pub revoked_at_ms: Option<i64>,
    #[serde(rename = "lastUsedAtMs", skip_serializing_if = "Option::is_none")]
    pub last_used_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AdminMcpTokenPublic {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub kind: AdminMcpTokenKind,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "expiresAtMs", skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    #[serde(rename = "revokedAtMs", skip_serializing_if = "Option::is_none")]
    pub revoked_at_ms: Option<i64>,
    #[serde(rename = "lastUsedAtMs", skip_serializing_if = "Option::is_none")]
    pub last_used_at_ms: Option<i64>,
    pub active: bool,
    pub expired: bool,
}

#[derive(Debug, Deserialize)]
pub struct IssueAdminMcpTokenInput {
    pub name: String,
    pub kind: AdminMcpTokenKind,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IssueAdminMcpTokenResponse {
    pub entry: AdminMcpTokenPublic,
    /// Plaintext bearer; shown once at issuance.
    pub token: String,
    #[serde(rename = "mcpEndpointPath")]
    pub mcp_endpoint_path: String,
    #[serde(rename = "mcpTransport")]
    pub mcp_transport: &'static str,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[must_use]
pub fn hash_admin_mcp_token(plain: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plain.as_bytes());
    hex::encode(hasher.finalize())
}

fn random_secret_hex() -> String {
    let mut buf = [0u8; 24];
    getrandom::getrandom(&mut buf).expect("getrandom");
    hex::encode(buf)
}

fn normalize_token_id(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 64 {
        return None;
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(s.to_string())
}

fn allocate_token_id(existing: &[AdminMcpTokenEntry]) -> String {
    let base = format!("amt-{}", now_ms());
    if existing.iter().any(|e| e.id == base) {
        format!("{base}-2")
    } else {
        base
    }
}

pub fn admin_mcp_tokens(settings: &GatewayGlobalSettingsStore) -> &[AdminMcpTokenEntry] {
    &settings.admin_mcp_tokens
}

#[must_use]
pub fn entry_is_active(entry: &AdminMcpTokenEntry, now: i64) -> bool {
    if entry.revoked_at_ms.is_some() {
        return false;
    }
    if let Some(exp) = entry.expires_at_ms {
        if now >= exp {
            return false;
        }
    }
    true
}

#[must_use]
pub fn entry_is_expired(entry: &AdminMcpTokenEntry, now: i64) -> bool {
    entry
        .expires_at_ms
        .is_some_and(|exp| now >= exp && entry.revoked_at_ms.is_none())
}

#[must_use]
pub fn to_public(entry: &AdminMcpTokenEntry, now: i64) -> AdminMcpTokenPublic {
    AdminMcpTokenPublic {
        id: entry.id.clone(),
        name: entry.name.clone(),
        note: entry.note.clone(),
        kind: entry.kind,
        created_at_ms: entry.created_at_ms,
        expires_at_ms: entry.expires_at_ms,
        revoked_at_ms: entry.revoked_at_ms,
        last_used_at_ms: entry.last_used_at_ms,
        active: entry_is_active(entry, now),
        expired: entry_is_expired(entry, now),
    }
}

pub fn admin_mcp_tokens_public(settings: &GatewayGlobalSettingsStore) -> Vec<AdminMcpTokenPublic> {
    let now = now_ms();
    settings
        .admin_mcp_tokens
        .iter()
        .map(|e| to_public(e, now))
        .collect()
}

pub async fn issue_admin_mcp_token(
    db: &GatewaySessionDb,
    input: IssueAdminMcpTokenInput,
) -> Result<IssueAdminMcpTokenResponse, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let note = input
        .note
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let id = allocate_token_id(&settings.admin_mcp_tokens);
    let now = now_ms();
    let expires_at_ms = match input.kind {
        AdminMcpTokenKind::Temporary => Some(now.saturating_add(TEMPORARY_TTL_MS)),
        AdminMcpTokenKind::Permanent => None,
    };
    let secret = random_secret_hex();
    let plain = format!("{TOKEN_PREFIX}{id}_{secret}");
    let entry = AdminMcpTokenEntry {
        id: id.clone(),
        name: name.to_string(),
        note,
        kind: input.kind,
        token_hash: hash_admin_mcp_token(&plain),
        created_at_ms: now,
        expires_at_ms,
        revoked_at_ms: None,
        last_used_at_ms: None,
    };
    settings.admin_mcp_tokens.push(entry.clone());
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;
    Ok(IssueAdminMcpTokenResponse {
        entry: to_public(&entry, now),
        token: plain,
        mcp_endpoint_path: ADMIN_MCP_HTTP_PATH.to_string(),
        mcp_transport: "streamable-http",
    })
}

pub async fn revoke_admin_mcp_token(db: &GatewaySessionDb, token_id: &str) -> Result<bool, String> {
    let id = normalize_token_id(token_id).ok_or_else(|| "invalid token id".to_string())?;
    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let Some(idx) = settings.admin_mcp_tokens.iter().position(|e| e.id == id) else {
        return Ok(false);
    };
    let now = now_ms();
    settings.admin_mcp_tokens[idx].revoked_at_ms = Some(now);
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

/// Validates bearer plaintext; updates `lastUsedAtMs` on success.
pub async fn verify_admin_mcp_token(
    db: &GatewaySessionDb,
    plain: &str,
) -> Result<AdminMcpTokenEntry, String> {
    let plain = plain.trim();
    if plain.is_empty() {
        return Err("missing token".into());
    }
    let hash = hash_admin_mcp_token(plain);
    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let now = now_ms();
    let idx = settings
        .admin_mcp_tokens
        .iter()
        .position(|e| e.token_hash == hash)
        .ok_or_else(|| "invalid admin MCP token".to_string())?;
    let entry = settings.admin_mcp_tokens[idx].clone();
    if !entry_is_active(&entry, now) {
        return Err("admin MCP token expired or revoked".into());
    }
    settings.admin_mcp_tokens[idx].last_used_at_ms = Some(now);
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;
    Ok(entry)
}

#[must_use]
pub fn extract_bearer_token(authorization: Option<&str>) -> Option<String> {
    let raw = authorization?.trim();
    let rest = raw.strip_prefix("Bearer ")?;
    let tok = rest.trim();
    if tok.is_empty() {
        None
    } else {
        Some(tok.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_stable() {
        assert_eq!(
            hash_admin_mcp_token("camt_amt-1_abcd"),
            hash_admin_mcp_token("camt_amt-1_abcd")
        );
    }

    #[test]
    fn temporary_entry_expires() {
        let now = 1_000_000_i64;
        let entry = AdminMcpTokenEntry {
            id: "amt-1".into(),
            name: "t".into(),
            note: None,
            kind: AdminMcpTokenKind::Temporary,
            token_hash: "h".into(),
            created_at_ms: now,
            expires_at_ms: Some(now + TEMPORARY_TTL_MS),
            revoked_at_ms: None,
            last_used_at_ms: None,
        };
        assert!(entry_is_active(&entry, now));
        assert!(!entry_is_active(&entry, now + TEMPORARY_TTL_MS));
    }

    #[test]
    fn extract_bearer_parses_header() {
        assert_eq!(
            extract_bearer_token(Some("Bearer camt_x_y")).as_deref(),
            Some("camt_x_y")
        );
    }
}
