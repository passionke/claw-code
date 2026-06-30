//! Admin read-only e2b NAS view from process env (repo `.env` → gateway restart). Author: kejiqing

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::pool::interactive_backend::E2bNasApiSingleton;
use crate::pool::nas_host_root;

#[derive(Debug, Clone, Serialize)]
pub struct E2bNasSettingsPublic {
    #[serde(rename = "readOnly")]
    pub read_only: bool,
    #[serde(rename = "nasHostMount")]
    pub nas_host_mount: String,
    #[serde(rename = "e2bNasServer")]
    pub e2b_nas_server: String,
    #[serde(rename = "e2bNasExport")]
    pub e2b_nas_export: String,
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

fn env_e2b_nas_server() -> Option<String> {
    std::env::var("CLAW_E2B_NAS_SERVER")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_e2b_nas_export() -> Option<String> {
    std::env::var("CLAW_E2B_NAS_EXPORT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Snapshot for Admin `GET /v1/gateway/global-settings` → `e2bNas` (env-only, not PG).
#[must_use]
pub fn e2b_nas_settings_public(work_root: &Path) -> E2bNasSettingsPublic {
    let nas_host_mount = env_nas_host_mount().unwrap_or_default();
    let e2b_nas_server = env_e2b_nas_server().unwrap_or_default();
    let e2b_nas_export = env_e2b_nas_export().unwrap_or_default();
    let gateway_work_root = gateway_work_root_from_env();
    let resolved = nas_host_root(work_root, None);
    let path_exists = resolved.exists() && resolved.is_dir();
    let has_proj_tree = if path_exists {
        Some(resolved.join("proj_1").exists())
    } else {
        None
    };
    let nas_api_enabled = E2bNasApiSingleton::enabled_from_env();
    E2bNasSettingsPublic {
        read_only: true,
        nas_host_mount,
        e2b_nas_server,
        e2b_nas_export,
        configured: nas_api_enabled,
        gateway_work_root: gateway_work_root.display().to_string(),
        nas_root_resolved: "claw-nas-api:/claw_ws".to_string(),
        layout_active: nas_api_enabled,
        path_exists,
        nas_api_enabled,
        has_proj_tree,
    }
}
