//! OVS project workspace metadata. Author: kejiqing

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::gateway_fc_ovs_settings::{self, workspace_folder_path, workspace_folder_url};
use crate::pool::interactive_backend::{ovs_backend_is_fc, FcProjWarmPool};
use crate::pool::proj_work_dir;
use crate::session_db::GatewaySessionDb;
use crate::session_terminal_api;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OvsWorkspaceResponse {
    pub proj_id: i64,
    /// Path inside the OVS container (`CLAW_OVS_MOUNT_ROOT` or fc `/claw_ws`).
    pub workspace_folder: String,
    /// Path on the gateway host under `work_root` (compose mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_path: Option<String>,
    /// Default agent session id for `@claw` in OVS (`ovs-{projId}`).
    pub agent_session_id: String,
    /// FC singleton OVS base URL (`http://3000-{sandboxId}.{domain}/ovs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ovs_url: Option<String>,
    /// Full browser URL including `?folder=…` (fc: direct e2b traffic, not gateway proxy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ovs_folder_url: Option<String>,
    /// Self-hosted: add this line to `/etc/hosts` once per OVS sandbox recreate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ovs_browser_hosts_line: Option<String>,
    /// `compose` | `fc`.
    pub ovs_backend: String,
}

#[derive(Debug)]
pub struct OvsApiError {
    status: StatusCode,
    message: String,
}

impl OvsApiError {
    #[must_use]
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl axum::response::IntoResponse for OvsApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

#[derive(Clone)]
pub struct OvsApiContext {
    pub work_root: PathBuf,
    /// Container mount root for OVS (compose `/home/workspace`; fc `/claw_ws`).
    pub ovs_mount_root: String,
}

#[must_use]
pub fn ovs_api_context(work_root: PathBuf) -> OvsApiContext {
    let ovs_mount_root = if ovs_backend_is_fc() {
        std::env::var("CLAW_OVS_MOUNT_ROOT")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/claw_ws".to_string())
    } else {
        std::env::var("CLAW_OVS_MOUNT_ROOT")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/home/workspace".to_string())
    };
    OvsApiContext {
        work_root,
        ovs_mount_root,
    }
}

pub fn ovs_agent_session_id(proj_id: i64) -> String {
    format!("ovs-{proj_id}")
}

/// OVS Chat panel record key (`gateway_sessions` / `gateway_turns` only; no worker lease).
#[must_use]
pub fn ovs_chat_record_session_id(proj_id: i64, chat_key: &str) -> String {
    let raw = chat_key.trim();
    if raw.is_empty() {
        return ovs_agent_session_id(proj_id);
    }
    let slug = crate::session_merge::sessions_directory_segment(raw);
    let slug = if slug.len() > 48 {
        slug[..16].to_string()
    } else {
        slug
    };
    format!("ovs-chat-{proj_id}-{slug}")
}

pub fn ovs_workspace_folder(ctx: &OvsApiContext, proj_id: i64) -> String {
    format!(
        "{}/proj_{proj_id}/home",
        ctx.ovs_mount_root.trim_end_matches('/')
    )
}

pub async fn get_ovs_workspace(
    ctx: OvsApiContext,
    session_db: &GatewaySessionDb,
    fc_warm: Option<&FcProjWarmPool>,
    proj_id: i64,
) -> Result<Json<OvsWorkspaceResponse>, OvsApiError> {
    if proj_id < 1 {
        return Err(OvsApiError::new(
            StatusCode::BAD_REQUEST,
            "projId must be >= 1",
        ));
    }

    if ovs_backend_is_fc() {
        if let Some(pool) = fc_warm {
            pool.ensure_warm(proj_id)
                .await
                .map_err(|e| OvsApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
        }
        let base_url = gateway_fc_ovs_settings::load_fc_ovs_base_url(session_db)
            .await
            .map_err(|e| OvsApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or_else(|| {
                OvsApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "OVS not configured — run deploy/stack/lib/fc-ovs-up.sh",
                )
            })?;
        let workspace_folder = workspace_folder_path(proj_id);
        let ovs_folder_url = workspace_folder_url(&base_url, proj_id);
        return Ok(Json(OvsWorkspaceResponse {
            proj_id,
            workspace_folder,
            host_path: None,
            agent_session_id: ovs_agent_session_id(proj_id),
            ovs_url: Some(base_url),
            ovs_folder_url: Some(ovs_folder_url),
            ovs_browser_hosts_line: None,
            ovs_backend: "fc".into(),
        }));
    }

    session_terminal_api::materialize_ovs_proj_workspace(session_db, &ctx.work_root, proj_id)
        .await
        .map_err(|e| OvsApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let host_path = proj_work_dir(&ctx.work_root, proj_id).join("home");
    Ok(Json(OvsWorkspaceResponse {
        proj_id,
        workspace_folder: ovs_workspace_folder(&ctx, proj_id),
        host_path: Some(host_path.display().to_string()),
        agent_session_id: ovs_agent_session_id(proj_id),
        ovs_url: None,
        ovs_folder_url: None,
        ovs_browser_hosts_line: None,
        ovs_backend: "compose".into(),
    }))
}

/// Merge `claw.projId` into a `.vscode/settings.json` path (create parents as needed).
async fn merge_claw_proj_id_settings(settings_path: &Path, proj_id: i64) -> Result<(), String> {
    if let Some(parent) = settings_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let mut cfg: Map<String, Value> = if settings_path.is_file() {
        let raw = tokio::fs::read_to_string(settings_path)
            .await
            .map_err(|e| format!("read {}: {e}", settings_path.display()))?;
        serde_json::from_str(&raw)
            .unwrap_or_else(|_| json!({}))
            .as_object()
            .cloned()
            .unwrap_or_default()
    } else {
        Map::new()
    };
    cfg.insert("claw.projId".to_string(), json!(proj_id));
    let body =
        serde_json::to_string_pretty(&cfg).map_err(|e| format!("serialize settings: {e}"))?;
    tokio::fs::write(settings_path, format!("{body}\n"))
        .await
        .map_err(|e| format!("write {}: {e}", settings_path.display()))?;
    Ok(())
}

/// Writes `proj_N/home/.vscode/settings.json` with authoritative `claw.projId` (Gateway contract).
pub async fn ensure_proj_claw_settings(proj_dir: &Path, proj_id: i64) -> Result<(), String> {
    let settings_path = proj_dir.join("home").join(".vscode").join("settings.json");
    merge_claw_proj_id_settings(&settings_path, proj_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ovs_chat_record_session_id_stable_and_distinct() {
        let a = ovs_chat_record_session_id(2, "chat-aaaa");
        let b = ovs_chat_record_session_id(2, "chat-aaaa");
        let c = ovs_chat_record_session_id(2, "chat-bbbb");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("ovs-chat-2-"));
        assert_eq!(ovs_agent_session_id(2), "ovs-2");
        assert_ne!(a, ovs_agent_session_id(2));
    }
}
