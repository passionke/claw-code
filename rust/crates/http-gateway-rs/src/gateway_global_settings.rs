//! Gateway-wide settings (not per `ds_id`): PAT vault for Git push, etc. Author: kejiqing

use runtime::builtin_system_prompt_scaffold_default;
use serde::{Deserialize, Serialize};

use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitPatEntry {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayGlobalSettingsPublic {
    #[serde(rename = "gitPats", default)]
    pub git_pats: Vec<GitPatPublic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitPatPublic {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    #[serde(rename = "tokenSet")]
    pub token_set: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayGlobalSettingsStore {
    #[serde(rename = "gitPats", default)]
    git_pats: Vec<GitPatEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GitPatTokensStore {
    #[serde(default)]
    pub tokens: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct PutGitPatInput {
    /// Omit to create; must be unique when provided.
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub note: Option<String>,
    /// Omit on update to keep existing token.
    #[serde(default)]
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GatewayGlobalSettingsResponse {
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    #[serde(rename = "gitPats")]
    pub git_pats: Vec<GitPatPublic>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn normalize_pat_id(raw: &str) -> Option<String> {
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

fn allocate_pat_id(existing: &[GitPatEntry]) -> String {
    let base = format!("pat-{}", now_ms());
    if existing.iter().any(|p| p.id == base) {
        format!("{base}-2")
    } else {
        base
    }
}

fn parse_settings_store(v: &serde_json::Value) -> GatewayGlobalSettingsStore {
    serde_json::from_value(v.clone()).unwrap_or_default()
}

fn parse_tokens_store(v: &serde_json::Value) -> GitPatTokensStore {
    if v.is_object() && v.get("tokens").is_none() {
        GitPatTokensStore {
            tokens: serde_json::from_value(v.clone()).unwrap_or_default(),
        }
    } else {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
}

fn tokens_to_json(store: &GitPatTokensStore) -> serde_json::Value {
    serde_json::to_value(&store.tokens).unwrap_or_else(|_| serde_json::json!({}))
}

pub async fn get_gateway_global_settings(
    db: &GatewaySessionDb,
) -> Result<(GatewayGlobalSettingsStore, GitPatTokensStore, i64), sqlx::Error> {
    let (settings_v, tokens_v, updated_at_ms) = db.get_gateway_global_settings_raw().await?;
    Ok((
        parse_settings_store(&settings_v),
        parse_tokens_store(&tokens_v),
        updated_at_ms,
    ))
}

pub async fn save_gateway_global_settings(
    db: &GatewaySessionDb,
    settings: &GatewayGlobalSettingsStore,
    tokens: &GitPatTokensStore,
    updated_at_ms: i64,
) -> Result<(), sqlx::Error> {
    let settings_v =
        serde_json::to_value(settings).unwrap_or_else(|_| serde_json::json!({"gitPats":[]}));
    db.save_gateway_global_settings_raw(&settings_v, &tokens_to_json(tokens), updated_at_ms)
        .await
}

pub async fn load_public(
    db: &GatewaySessionDb,
) -> Result<GatewayGlobalSettingsPublic, sqlx::Error> {
    let (settings, tokens, _) = get_gateway_global_settings(db).await?;
    Ok(to_public(&settings, &tokens))
}

pub async fn load_response(
    db: &GatewaySessionDb,
) -> Result<GatewayGlobalSettingsResponse, sqlx::Error> {
    let (settings, tokens, updated_at_ms) = get_gateway_global_settings(db).await?;
    Ok(GatewayGlobalSettingsResponse {
        updated_at_ms,
        git_pats: to_public(&settings, &tokens).git_pats,
    })
}

pub async fn validate_git_sync_json_with_global(
    db: &GatewaySessionDb,
    v: &serde_json::Value,
) -> Result<(), String> {
    let sync = crate::project_git_sync::parse_git_sync_json(v);
    let tokens = load_git_pat_tokens(db).await.map_err(|e| e.to_string())?;
    let resolved = crate::project_git_sync::resolve_git_sync_credentials(&sync, &tokens.tokens);
    crate::project_git_sync::validate_git_sync_resolved(&resolved)
}

pub async fn upsert_git_pat(
    db: &GatewaySessionDb,
    input: PutGitPatInput,
) -> Result<GitPatPublic, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let note = input
        .note
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let (mut settings, mut tokens, _updated_at_ms) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let id = if let Some(raw) = input.id.as_deref() {
        normalize_pat_id(raw).ok_or_else(|| "invalid pat id".to_string())?
    } else {
        allocate_pat_id(&settings.git_pats)
    };
    let now = now_ms();
    if let Some(idx) = settings.git_pats.iter().position(|p| p.id == id) {
        let entry = &mut settings.git_pats[idx];
        entry.name = name.to_string();
        entry.note = note;
        entry.updated_at_ms = now;
        if let Some(tok) = input
            .token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            tokens.tokens.insert(id.clone(), tok.to_string());
        }
    } else {
        let tok = input
            .token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "token is required for new PAT".to_string())?;
        settings.git_pats.push(GitPatEntry {
            id: id.clone(),
            name: name.to_string(),
            note,
            created_at_ms: now,
            updated_at_ms: now,
        });
        tokens.tokens.insert(id.clone(), tok.to_string());
    }
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;
    let public = to_public(&settings, &tokens);
    public
        .git_pats
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "pat missing after save".to_string())
}

pub async fn delete_git_pat(db: &GatewaySessionDb, pat_id: &str) -> Result<bool, String> {
    let id = normalize_pat_id(pat_id).ok_or_else(|| "invalid pat id".to_string())?;
    let (mut settings, mut tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let before = settings.git_pats.len();
    settings.git_pats.retain(|p| p.id != id);
    tokens.tokens.remove(&id);
    if settings.git_pats.len() == before {
        return Ok(false);
    }
    let now = now_ms();
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;
    Ok(true)
}

#[must_use]
pub fn resolve_git_pat_token(pat_id: Option<&str>, tokens: &GitPatTokensStore) -> Option<String> {
    let id = pat_id?.trim();
    if id.is_empty() {
        return None;
    }
    tokens.tokens.get(id).cloned()
}

pub async fn load_git_pat_tokens(db: &GatewaySessionDb) -> Result<GitPatTokensStore, sqlx::Error> {
    let (_, tokens, _) = get_gateway_global_settings(db).await?;
    Ok(tokens)
}

/// Builtin system prompt scaffold from PG (not exposed on Admin API). Author: kejiqing
pub async fn load_system_prompt_default(db: &GatewaySessionDb) -> Result<String, sqlx::Error> {
    let (text, _) = db.get_gateway_system_prompt_default().await?;
    Ok(if text.trim().is_empty() {
        builtin_system_prompt_scaffold_default()
    } else {
        text
    })
}

#[must_use]
pub fn to_public(
    settings: &GatewayGlobalSettingsStore,
    tokens: &GitPatTokensStore,
) -> GatewayGlobalSettingsPublic {
    GatewayGlobalSettingsPublic {
        git_pats: settings
            .git_pats
            .iter()
            .map(|p| GitPatPublic {
                id: p.id.clone(),
                name: p.name.clone(),
                note: p.note.clone(),
                created_at_ms: p.created_at_ms,
                updated_at_ms: p.updated_at_ms,
                token_set: tokens.tokens.contains_key(&p.id),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pat_id_accepts_slug() {
        assert_eq!(
            normalize_pat_id("pat-github-1").as_deref(),
            Some("pat-github-1")
        );
    }
}
