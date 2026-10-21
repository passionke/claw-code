//! Project workspace materialize (proj home + claw.projId settings).
//! Former terminal / agent/ws lease API removed with OVS + web CLI. Author: kejiqing

use std::path::Path;

use serde_json::{json, Map, Value};

use crate::gateway_global_settings;
use crate::pool::gateway_proj_work_dir;
use crate::project_config_apply;
use crate::project_config_draft;
use crate::session_db::GatewaySessionDb;

async fn materialize_proj_home(
    session_db: &GatewaySessionDb,
    proj_dir: &Path,
    proj_id: i64,
) -> Result<(), String> {
    let row = project_config_draft::row_for_materialize(session_db, proj_id)
        .await
        .map_err(|e| format!("load project_config: {e}"))?;
    let Some(row) = row else {
        tokio::fs::create_dir_all(proj_dir.join("home"))
            .await
            .map_err(|e| format!("mkdir proj home: {e}"))?;
        write_proj_vscode_settings(proj_dir, proj_id).await?;
        return Ok(());
    };
    tokio::fs::create_dir_all(proj_dir.join(".claw"))
        .await
        .map_err(|e| format!("mkdir proj .claw: {e}"))?;
    let scaffold = gateway_global_settings::load_system_prompt_default(session_db)
        .await
        .map_err(|e| format!("load system prompt scaffold: {e}"))?;
    project_config_apply::apply_if_needed(proj_dir, &row, false, &scaffold)
        .await
        .map_err(|e| format!("apply project_config: {e}"))?;
    project_config_apply::apply_interactive_ds_layout_under_home(proj_dir, &row, &scaffold)
        .await
        .map_err(|e| format!("apply interactive /claw_ds layout: {e}"))?;
    project_config_apply::link_claw_compat_symlinks(proj_dir)
        .await
        .map_err(|e| format!("link claw compat symlinks: {e}"))?;
    write_proj_vscode_settings(proj_dir, proj_id).await?;
    Ok(())
}

/// Full PG materialize for project workspace (`proj_N/home` + interactive layout). Author: kejiqing
pub async fn materialize_proj_workspace(
    session_db: &GatewaySessionDb,
    work_root: &Path,
    proj_id: i64,
) -> Result<(), String> {
    let proj_dir = gateway_proj_work_dir(work_root, proj_id)?;
    materialize_proj_home(session_db, &proj_dir, proj_id).await
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

async fn write_proj_vscode_settings(proj_dir: &Path, proj_id: i64) -> Result<(), String> {
    ensure_proj_claw_settings(proj_dir, proj_id).await
}
