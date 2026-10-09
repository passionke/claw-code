//! Dynamic Agent engine artifacts installed when an e2b Worker starts. Author: kejiqing

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEngineEntry {
    pub r#ref: String,
    pub digest: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEngines {
    #[serde(default)]
    pub engines: BTreeMap<String, AgentEngineEntry>,
}

fn normalize_engine_id(raw: &str) -> Result<String, String> {
    let id = raw.trim().to_ascii_lowercase();
    let mut chars = id.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!("invalid Agent engine ID: {raw}"));
    }
    Ok(id)
}

fn normalize_entry(mut entry: AgentEngineEntry) -> Result<AgentEngineEntry, String> {
    entry.r#ref = entry.r#ref.trim().to_string();
    if !entry.r#ref.starts_with("http://") && !entry.r#ref.starts_with("https://") {
        return Err("Agent engine ref must be an http(s) raw tar.gz URL".into());
    }
    if !entry.r#ref.ends_with(".tar.gz") || entry.r#ref.contains(char::is_whitespace) {
        return Err("Agent engine ref must be a tar.gz URL without whitespace".into());
    }
    entry.digest = entry.digest.trim().to_ascii_lowercase();
    let Some(hex) = entry.digest.strip_prefix("sha256:") else {
        return Err("Agent engine digest must use sha256:<64 hex>".into());
    };
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Agent engine digest must use sha256:<64 hex>".into());
    }
    Ok(entry)
}

pub async fn load_agent_engines(db: &GatewaySessionDb) -> Result<AgentEngines, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings.agent_engines)
}

pub async fn put_agent_engines(
    db: &GatewaySessionDb,
    input: AgentEngines,
) -> Result<AgentEngines, String> {
    let mut engines = BTreeMap::new();
    for (raw_id, raw_entry) in input.engines {
        let id = normalize_engine_id(&raw_id)?;
        if engines
            .insert(id.clone(), normalize_entry(raw_entry)?)
            .is_some()
        {
            return Err(format!("duplicate normalized Agent engine ID: {id}"));
        }
    }

    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    settings.agent_engines = AgentEngines { engines };
    let now = chrono::Utc::now().timestamp_millis();
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;
    Ok(settings.agent_engines)
}

pub fn agent_engines_from_value(value: &serde_json::Value) -> AgentEngines {
    serde_json::from_value(
        value
            .get("agentEngines")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({})),
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> AgentEngineEntry {
        AgentEngineEntry {
            r#ref: "https://raw.example/engines/future-v1.tar.gz".into(),
            digest: format!("sha256:{}", "a".repeat(64)),
        }
    }

    #[test]
    fn accepts_dynamic_engine_id() {
        assert_eq!(
            normalize_engine_id(" Future.Engine_2 ").unwrap(),
            "future.engine_2"
        );
        assert!(normalize_entry(entry()).is_ok());
    }

    #[test]
    fn rejects_invalid_ref_and_digest() {
        let mut invalid = entry();
        invalid.r#ref = "registry.example/engine:v1".into();
        assert!(normalize_entry(invalid).is_err());
        let mut invalid = entry();
        invalid.digest = "sha256:1234".into();
        assert!(normalize_entry(invalid).is_err());
    }

    #[test]
    fn dynamic_engine_map_round_trips_through_settings_json() {
        let value = serde_json::json!({
            "agentEngines": {
                "engines": {
                    "future-engine": entry()
                }
            }
        });
        let loaded = agent_engines_from_value(&value);
        assert_eq!(loaded.engines.get("future-engine"), Some(&entry()));
        assert_eq!(
            serde_json::to_value(loaded).unwrap()["engines"]["future-engine"]["ref"],
            entry().r#ref
        );
    }
}
