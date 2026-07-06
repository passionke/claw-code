//! OVS workspace URLs derived from relaxed worker built-in openvscode-server. Author: kejiqing

use claw_e2b_sandbox_client::{ovs_folder_url, ovs_workspace_folder, E2bSandboxHandle};

use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;

/// Deprecated: legacy singleton OVS mount root (`/claw_ws`). New code uses [`ovs_workspace_folder`].
pub const OVS_WORKSPACE_ROOT: &str = claw_e2b_sandbox_client::GUEST_CLAW_WS;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, serde::Serialize)]
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
    #[serde(rename = "deprecated")]
    pub deprecated: bool,
    #[serde(rename = "migrationNote", skip_serializing_if = "Option::is_none")]
    pub migration_note: Option<String>,
}

/// Built-in OVS base URL from a relaxed project worker handle.
#[must_use]
pub fn ovs_base_url_from_handle(handle: &E2bSandboxHandle) -> Option<String> {
    handle
        .ovs_base_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_string)
}

/// Browser folder URL (`?folder=/claw_ds`) from a relaxed project worker handle.
#[must_use]
pub fn ovs_folder_url_from_handle(handle: &E2bSandboxHandle) -> Option<String> {
    ovs_base_url_from_handle(handle).map(|base| ovs_folder_url(&base))
}

#[must_use]
pub fn workspace_folder_path() -> &'static str {
    ovs_workspace_folder()
}

pub async fn e2b_ovs_settings_public(
    db: &GatewaySessionDb,
) -> Result<E2bOvsSettingsPublic, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    let s = &settings.e2b_ovs;
    Ok(E2bOvsSettingsPublic {
        template_id: s.template_id.clone(),
        effective_template_id: "claw-worker-relaxed (built-in)".into(),
        base_url: s.base_url.clone(),
        sandbox_id: s.sandbox_id.clone(),
        updated_at_ms: s.updated_at_ms,
        configured: s.configured(),
        deprecated: true,
        migration_note: Some(
            "OVS runs inside relaxed project workers; use GET /v1/projects/{id}/ovs/workspace"
                .into(),
        ),
    })
}

pub async fn load_e2b_ovs_settings(db: &GatewaySessionDb) -> Result<E2bOvsSettings, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings.e2b_ovs)
}

/// Legacy singleton template id (deprecated).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ovs_folder_url_from_handle_uses_claw_ds() {
        let handle = E2bSandboxHandle {
            sandbox_id: "sbx_abc".into(),
            sandbox_domain: "supone.top".into(),
            envd_access_token: None,
            traffic_access_token: None,
            ttyd_public_host: "7681-sbx_abc.supone.top".into(),
            ttyd_use_tls: false,
            ovs_public_host: Some("3000-sbx_abc.supone.top".into()),
            ovs_base_url: Some("http://3000-sbx_abc.supone.top/ovs".into()),
        };
        let url = ovs_folder_url_from_handle(&handle).expect("url");
        assert!(url.contains("folder=/claw_ds"));
        assert!(!url.contains("/claw_ws/"));
        assert_eq!(workspace_folder_path(), "/claw_ds");
    }
}
