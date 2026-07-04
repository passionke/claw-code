//! e2b claw-nas-api singleton endpoint in PG. Author: kejiqing
//!
//! Gateway ensures the singleton on startup and registers it for lease ticker renewal;
//! `baseUrl` is read here on every NAS call.

use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct E2bNasApiSettings {
    #[serde(rename = "templateId", default)]
    pub template_id: Option<String>,
    #[serde(rename = "baseUrl", default)]
    pub base_url: Option<String>,
    #[serde(rename = "sandboxId", default)]
    pub sandbox_id: Option<String>,
    #[serde(rename = "updatedAtMs", default)]
    pub updated_at_ms: i64,
}

impl E2bNasApiSettings {
    #[must_use]
    pub fn configured(&self) -> bool {
        self.base_url.as_ref().is_some_and(|u| !u.trim().is_empty())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct E2bNasApiSettingsPublic {
    #[serde(rename = "templateId", skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(rename = "effectiveTemplateId")]
    pub effective_template_id: String,
    #[serde(rename = "baseUrl", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(rename = "sandboxId", skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    pub online: bool,
}

/// PG `e2bNasApi.templateId` → env `CLAW_E2B_NAS_API_TEMPLATE` → `claw-nas-api`.
#[must_use]
pub fn e2b_nas_api_template_from_env() -> String {
    std::env::var("CLAW_E2B_NAS_API_TEMPLATE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claw-nas-api".into())
}

pub async fn load_e2b_nas_api_template_id(db: &GatewaySessionDb) -> Result<String, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings
        .e2b_nas_api
        .template_id
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(e2b_nas_api_template_from_env))
}

pub async fn e2b_nas_api_settings_public(
    db: &GatewaySessionDb,
) -> Result<E2bNasApiSettingsPublic, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    let effective_template_id = load_e2b_nas_api_template_id(db).await?;
    let s = &settings.e2b_nas_api;
    Ok(E2bNasApiSettingsPublic {
        template_id: s.template_id.clone(),
        effective_template_id,
        base_url: s.base_url.clone(),
        sandbox_id: s.sandbox_id.clone(),
        updated_at_ms: s.updated_at_ms,
        online: s.configured(),
    })
}

pub async fn load_e2b_nas_api_settings(
    db: &GatewaySessionDb,
) -> Result<E2bNasApiSettings, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings.e2b_nas_api)
}

pub async fn load_e2b_nas_api_base_url(
    db: &GatewaySessionDb,
) -> Result<Option<String>, sqlx::Error> {
    let s = load_e2b_nas_api_settings(db).await?;
    Ok(s.base_url
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty()))
}
