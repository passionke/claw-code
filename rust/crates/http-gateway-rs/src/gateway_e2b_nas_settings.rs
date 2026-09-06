//! Admin read-only e2b NAS view (e2b GET /health + nas-api gate). Author: kejiqing

use claw_e2b_sandbox_client::E2bNasPlatform;
use serde::Serialize;

use crate::pool::interactive_backend::E2bNasApiSingleton;

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct E2bNasSettingsPublic {
    #[serde(rename = "readOnly")]
    pub read_only: bool,
    /// e2b host bind root from `GET /health` → `nas.hostMountRoot` (not Gateway env).
    #[serde(rename = "e2bHostMountRoot")]
    pub e2b_host_mount_root: String,
    #[serde(rename = "e2bNasReady")]
    pub e2b_nas_ready: bool,
    #[serde(rename = "sandboxInject", skip_serializing_if = "Option::is_none")]
    pub sandbox_inject: Option<String>,
    #[serde(rename = "mountSource", skip_serializing_if = "Option::is_none")]
    pub mount_source: Option<String>,
    #[serde(rename = "nasApiEnabled")]
    pub nas_api_enabled: bool,
    #[serde(rename = "layoutActive")]
    pub layout_active: bool,
    pub configured: bool,
}

/// Snapshot for Admin `GET /v1/gateway/global-settings` → `e2bNas`.
#[must_use]
pub fn e2b_nas_settings_public(platform: Option<&E2bNasPlatform>) -> E2bNasSettingsPublic {
    let nas_api_enabled = E2bNasApiSingleton::enabled_from_env();
    let (e2b_host_mount_root, e2b_nas_ready, sandbox_inject, mount_source) = match platform {
        Some(p) => (
            p.host_mount_root.clone().unwrap_or_default(),
            p.ready && p.uses_host_bind_inject(),
            p.sandbox_inject.clone(),
            Some(p.mount_source.clone()),
        ),
        None => (String::new(), false, None, None),
    };
    let layout_active = nas_api_enabled && e2b_nas_ready;
    E2bNasSettingsPublic {
        read_only: true,
        e2b_host_mount_root,
        e2b_nas_ready,
        sandbox_inject,
        mount_source,
        nas_api_enabled,
        layout_active,
        configured: layout_active,
    }
}
