//! e2b OVS singleton URLs persisted by gateway lifecycle in PG. Author: kejiqing

use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;

pub const OVS_WORKSPACE_ROOT: &str = claw_e2b_sandbox_client::GUEST_CLAW_WS;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct E2bOvsSettings {
    #[serde(rename = "templateId", default)]
    pub template_id: Option<String>,
    #[serde(rename = "baseUrl", default)]
    pub base_url: Option<String>,
    #[serde(rename = "sandboxId", default)]
    pub sandbox_id: Option<String>,
    #[serde(rename = "updatedAtMs", default)]
    pub updated_at_ms: i64,
}

impl E2bOvsSettings {
    #[must_use]
    pub fn configured(&self) -> bool {
        self.updated_at_ms > 0 && self.base_url.as_ref().is_some_and(|u| !u.trim().is_empty())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct E2bOvsSettingsPublic {
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
    pub configured: bool,
}

/// PG `e2bOvs.templateId` → env `CLAW_E2B_OVS_TEMPLATE` → `claw-ovs`.
#[must_use]
pub fn e2b_ovs_template_from_env() -> String {
    std::env::var("CLAW_E2B_OVS_TEMPLATE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claw-ovs".into())
}

pub async fn load_e2b_ovs_template_id(db: &GatewaySessionDb) -> Result<String, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings
        .e2b_ovs
        .template_id
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(e2b_ovs_template_from_env))
}

pub async fn e2b_ovs_settings_public(
    db: &GatewaySessionDb,
) -> Result<E2bOvsSettingsPublic, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    let effective_template_id = load_e2b_ovs_template_id(db).await?;
    let s = &settings.e2b_ovs;
    Ok(E2bOvsSettingsPublic {
        template_id: s.template_id.clone(),
        effective_template_id,
        base_url: s.base_url.clone(),
        sandbox_id: s.sandbox_id.clone(),
        updated_at_ms: s.updated_at_ms,
        configured: s.configured(),
    })
}

/// Folder URL for a project inside the singleton OVS.
#[must_use]
pub fn workspace_folder_url(base_url: &str, proj_id: i64) -> String {
    format!(
        "{}?folder={}/proj_{proj_id}/home",
        base_url.trim_end_matches('/'),
        OVS_WORKSPACE_ROOT
    )
}

#[must_use]
pub fn workspace_folder_path(proj_id: i64) -> String {
    format!("{OVS_WORKSPACE_ROOT}/proj_{proj_id}/home")
}

pub async fn load_e2b_ovs_settings(db: &GatewaySessionDb) -> Result<E2bOvsSettings, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings.e2b_ovs)
}

pub async fn load_e2b_ovs_base_url(db: &GatewaySessionDb) -> Result<Option<String>, sqlx::Error> {
    let s = load_e2b_ovs_settings(db).await?;
    Ok(s.base_url.filter(|u| !u.trim().is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_folder_url_format() {
        let url = workspace_folder_url("http://3000-sbx_abc.supone.top/ovs", 2);
        assert!(url.contains("proj_2/home"));
        assert!(url.starts_with("http://3000-sbx_abc.supone.top/ovs?folder="));
    }
}
