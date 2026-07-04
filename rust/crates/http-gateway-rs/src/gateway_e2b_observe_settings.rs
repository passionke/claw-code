//! e2b observe singleton template id in PG (`settings_json.e2bObserve`). Author: kejiqing

use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct E2bObserveSettings {
    #[serde(rename = "templateId", default)]
    pub template_id: Option<String>,
    #[serde(rename = "updatedAtMs", default)]
    pub updated_at_ms: i64,
}

impl E2bObserveSettings {
    #[must_use]
    pub fn configured(&self) -> bool {
        self.template_id
            .as_ref()
            .is_some_and(|t| !t.trim().is_empty())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct E2bObserveSettingsPublic {
    #[serde(rename = "templateId", skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(rename = "effectiveTemplateId")]
    pub effective_template_id: String,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    pub configured: bool,
}

/// PG `e2bObserve.templateId` → env `CLAW_E2B_OBSERVE_TEMPLATE` → `claw-observe`.
#[must_use]
pub fn e2b_observe_template_from_env() -> String {
    std::env::var("CLAW_E2B_OBSERVE_TEMPLATE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claw-observe".into())
}

pub async fn load_e2b_observe_settings(
    db: &GatewaySessionDb,
) -> Result<E2bObserveSettings, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings.e2b_observe)
}

pub async fn load_e2b_observe_template_id(db: &GatewaySessionDb) -> Result<String, sqlx::Error> {
    let settings = load_e2b_observe_settings(db).await?;
    Ok(settings
        .template_id
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(e2b_observe_template_from_env))
}

pub async fn e2b_observe_settings_public(
    db: &GatewaySessionDb,
) -> Result<E2bObserveSettingsPublic, sqlx::Error> {
    let settings = load_e2b_observe_settings(db).await?;
    let effective_template_id = load_e2b_observe_template_id(db).await?;
    let configured = settings.configured();
    Ok(E2bObserveSettingsPublic {
        template_id: settings.template_id,
        effective_template_id,
        updated_at_ms: settings.updated_at_ms,
        configured,
    })
}
