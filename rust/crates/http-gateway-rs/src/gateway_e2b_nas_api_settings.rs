//! e2b claw-nas-api singleton endpoint persisted by `e2b-nas-api-up.py` in PG. Author: kejiqing
//!
//! Mirrors the OVS/observe contract: the singleton is deployed out-of-band
//! (`./deploy/stack/gateway.sh nas-api-up`) and its endpoint is written to
//! `gateway_global_settings.settings_json.e2bNasApi`. The gateway is a pure
//! consumer — it reads `baseUrl` here and never creates the sandbox itself.

use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct E2bNasApiSettings {
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
