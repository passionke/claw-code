//! Admin read-only FC NAS view from process env (repo `.env` → gateway restart). Author: kejiqing

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::pool::interactive_backend::FcNasApiSingleton;
use crate::pool::nas_host_root;

#[derive(Debug, Clone, Serialize)]
pub struct FcNasSettingsPublic {
    #[serde(rename = "readOnly")]
    pub read_only: bool,
    #[serde(rename = "nasHostMount")]
    pub nas_host_mount: String,
    #[serde(rename = "fcNasServer")]
    pub fc_nas_server: String,
    #[serde(rename = "fcNasExport")]
    pub fc_nas_export: String,
    pub configured: bool,
    /// Gateway container work root (`CLAW_WORK_ROOT`).
    #[serde(rename = "gatewayWorkRoot")]
    pub gateway_work_root: String,
    /// Effective NAS root used for mkdir/symlink.
    #[serde(rename = "nasRootResolved")]
    pub nas_root_resolved: String,
    #[serde(rename = "layoutActive")]
    pub layout_active: bool,
    #[serde(rename = "pathExists")]
    pub path_exists: bool,
    #[serde(rename = "nasApiEnabled")]
    pub nas_api_enabled: bool,
    #[serde(rename = "hasProjTree", skip_serializing_if = "Option::is_none")]
    pub has_proj_tree: Option<bool>,
}

#[must_use]
pub fn gateway_work_root_from_env() -> PathBuf {
    std::env::var("CLAW_WORK_ROOT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/claw/workspace"))
}

fn env_nas_host_mount() -> Option<String> {
    std::env::var("CLAW_NAS_HOST_MOUNT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_fc_nas_server() -> Option<String> {
    std::env::var("CLAW_FC_NAS_SERVER")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_fc_nas_export() -> Option<String> {
    std::env::var("CLAW_FC_NAS_EXPORT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Snapshot for Admin `GET /v1/gateway/global-settings` → `fcNas` (env-only, not PG).
#[must_use]
pub fn fc_nas_settings_public(work_root: &Path) -> FcNasSettingsPublic {
    let nas_host_mount = env_nas_host_mount().unwrap_or_default();
    let fc_nas_server = env_fc_nas_server().unwrap_or_default();
    let fc_nas_export = env_fc_nas_export().unwrap_or_default();
    let gateway_work_root = gateway_work_root_from_env();
    let resolved = nas_host_root(work_root, None);
    let path_exists = resolved.exists() && resolved.is_dir();
    let has_proj_tree = if path_exists {
        Some(resolved.join("proj_1").exists() || resolved.join(".claw-fc-tools").exists())
    } else {
        None
    };
    let nas_api_enabled = FcNasApiSingleton::enabled_from_env();
    FcNasSettingsPublic {
        read_only: true,
        nas_host_mount,
        fc_nas_server,
        fc_nas_export,
        configured: nas_api_enabled,
        gateway_work_root: gateway_work_root.display().to_string(),
        nas_root_resolved: "claw-nas-api:/claw_ws".to_string(),
        layout_active: nas_api_enabled,
        path_exists,
        nas_api_enabled,
        has_proj_tree,
    }
}
