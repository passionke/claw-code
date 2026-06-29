//! FC/E2B NAS layout + materialize via claw-nas-api singleton or direct NAS bind. Author: kejiqing

use std::path::{Path, PathBuf};

use claw_fc_sandbox_client::{
    proj_home_rel, session_ds_symlink_target, session_rel, sessions_root_rel, tap_traces_rel,
    worker_rel, workers_root_rel,
};
use serde_json::json;

use crate::cluster_identity;
use crate::project_config_apply;
use crate::project_config_draft;
use crate::session_db::GatewaySessionDb;

use super::fc_nas_layout::fc_nas_layout_active;
use super::interactive_backend::FcNasApiSingleton;
use super::session_mount_ownership::ensure_session_tree_owned_for_worker_with_runtime_fallback;

#[derive(Clone)]
pub struct NasLayoutBackend {
    nas_api: Option<std::sync::Arc<FcNasApiSingleton>>,
    local_root: PathBuf,
    runtime_bin: String,
}

impl NasLayoutBackend {
    #[must_use]
    pub fn new(nas_api: Option<std::sync::Arc<FcNasApiSingleton>>, local_root: PathBuf) -> Self {
        let runtime_bin =
            std::env::var("CLAW_CONTAINER_RUNTIME").unwrap_or_else(|_| "podman".into());
        Self {
            nas_api,
            local_root,
            runtime_bin,
        }
    }

    #[must_use]
    pub fn uses_nas_api(&self) -> bool {
        self.nas_api.is_some()
    }

    #[must_use]
    pub fn active(&self) -> bool {
        self.uses_nas_api() || fc_nas_layout_active(&self.local_root)
    }

    #[must_use]
    pub fn local_root(&self) -> &Path {
        &self.local_root
    }

    pub fn cluster_id(&self) -> Result<String, String> {
        cluster_identity::gateway_cluster_id()
    }

    async fn mkdir_rel(&self, rel: &str) -> Result<(), String> {
        if let Some(api) = &self.nas_api {
            return api.mkdir(rel, true).await;
        }
        let path = self.local_root.join(rel);
        tokio::fs::create_dir_all(&path)
            .await
            .map_err(|e| format!("mkdir {}: {e}", path.display()))
    }

    async fn put_proj_home_file(
        &self,
        cluster_id: &str,
        proj_id: i64,
        rel_under_home: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let rel = rel_under_home.trim_start_matches('/');
        if let Some(api) = &self.nas_api {
            return api.put_proj_home_file(proj_id, rel, bytes).await;
        }
        let path = self
            .local_root
            .join(proj_home_rel(cluster_id, proj_id))
            .join(rel);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|e| format!("write {}: {e}", path.display()))
    }

    async fn symlink_rel(&self, rel: &str, target: &str) -> Result<(), String> {
        if let Some(api) = &self.nas_api {
            return api.symlink(rel, target).await;
        }
        let link_path = self.local_root.join(rel);
        super::fc_nas_layout::replace_session_path_local(&link_path).await?;
        #[cfg(unix)]
        {
            tokio::fs::symlink(target, &link_path)
                .await
                .map_err(|e| format!("symlink {} -> {target}: {e}", link_path.display()))?;
        }
        #[cfg(not(unix))]
        {
            return Err("NAS session symlink requires unix".into());
        }
        Ok(())
    }

    async fn unlink_rel(&self, rel: &str) -> Result<(), String> {
        if let Some(api) = &self.nas_api {
            return api.unlink(rel).await;
        }
        let path = self.local_root.join(rel);
        if path.exists() || path.is_symlink() {
            super::fc_nas_layout::remove_path_all_local(&path).await?;
        }
        Ok(())
    }

    pub async fn ensure_fc_proj_nas_roots(&self, proj_id: i64) -> Result<(), String> {
        let cluster_id = self.cluster_id()?;
        self.mkdir_rel(&sessions_root_rel(&cluster_id, proj_id))
            .await?;
        self.mkdir_rel(&workers_root_rel(&cluster_id, proj_id))
            .await?;
        self.mkdir_rel(&proj_home_rel(&cluster_id, proj_id))
            .await?;
        self.mkdir_rel(tap_traces_rel()).await?;
        Ok(())
    }

    pub async fn ensure_worker_root(&self, proj_id: i64, worker_id: &str) -> Result<(), String> {
        let cluster_id = self.cluster_id()?;
        self.mkdir_rel(&workers_root_rel(&cluster_id, proj_id))
            .await?;
        let wr = worker_rel(&cluster_id, proj_id, worker_id);
        self.mkdir_rel(&format!("{wr}/.claw")).await?;
        if self.nas_api.is_none() {
            let worker_abs = self.local_root.join(&wr);
            ensure_session_tree_owned_for_worker_with_runtime_fallback(
                &self.runtime_bin,
                &worker_abs,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn ensure_session_context(
        &self,
        proj_id: i64,
        session_segment: &str,
        _worker_id: &str,
    ) -> Result<(), String> {
        self.ensure_session_root(proj_id, session_segment).await
    }

    /// Real session directory with `.claw/`, `work/`, `ds → ../../home`.
    pub async fn ensure_session_root(
        &self,
        proj_id: i64,
        session_segment: &str,
    ) -> Result<(), String> {
        let cluster_id = self.cluster_id()?;
        self.mkdir_rel(&sessions_root_rel(&cluster_id, proj_id))
            .await?;
        let session_rel_path = session_rel(&cluster_id, proj_id, session_segment);
        if self.nas_api.is_none() {
            let session_abs = self.local_root.join(&session_rel_path);
            super::fc_nas_layout::replace_session_path_local(&session_abs).await?;
        }
        self.mkdir_rel(&format!("{session_rel_path}/.claw"))
            .await?;
        self.mkdir_rel(&format!("{session_rel_path}/work")).await?;
        let ds_rel = format!("{session_rel_path}/ds");
        if self.nas_api.is_some() {
            let _ = self.unlink_rel(&ds_rel).await;
        }
        self.symlink_rel(&ds_rel, session_ds_symlink_target())
            .await
    }

    /// PG project config → `proj_N/home` on NAS (via nas-api or local bind).
    pub async fn materialize_proj_workspace(
        &self,
        session_db: &GatewaySessionDb,
        proj_id: i64,
    ) -> Result<(), String> {
        let cluster_id = self.cluster_id()?;
        self.mkdir_rel(&proj_home_rel(&cluster_id, proj_id)).await?;
        let row = project_config_draft::row_for_materialize(session_db, proj_id)
            .await
            .map_err(|e| format!("load project_config: {e}"))?;
        let Some(row) = row else {
            self.write_proj_vscode_settings(&cluster_id, proj_id).await?;
            return Ok(());
        };
        let scaffold = crate::gateway_global_settings::load_system_prompt_default(session_db)
            .await
            .map_err(|e| format!("load system prompt scaffold: {e}"))?;
        let writes = project_config_apply::build_guest_materialize_writes(&row, &scaffold)
            .map_err(|e| format!("build guest materialize writes: {e}"))?;
        for write in writes {
            let rel_under_home = write.rel_path.to_string_lossy();
            self.put_proj_home_file(&cluster_id, proj_id, &rel_under_home, &write.bytes)
                .await?;
        }
        let proj_prefix = format!("{cluster_id}/proj_{proj_id}");
        self.mkdir_rel(&format!("{proj_prefix}/home/skills"))
            .await
            .ok();
        self.mkdir_rel(&format!("{proj_prefix}/home/.cursor/rules"))
            .await
            .ok();
        self.symlink_rel(&format!("{proj_prefix}/.claw/skills"), "../home/skills")
            .await
            .ok();
        self.symlink_rel(
            &format!("{proj_prefix}/.cursor/rules"),
            "../home/.cursor/rules",
        )
        .await
        .ok();
        self.write_proj_vscode_settings(&cluster_id, proj_id).await?;
        Ok(())
    }

    async fn write_proj_vscode_settings(
        &self,
        cluster_id: &str,
        proj_id: i64,
    ) -> Result<(), String> {
        let body = serde_json::to_string_pretty(&json!({ "claw.projId": proj_id }))
            .map_err(|e| format!("serialize vscode settings: {e}"))?
            + "\n";
        self.put_proj_home_file(cluster_id, proj_id, ".vscode/settings.json", body.as_bytes())
            .await
    }

    pub async fn prepare_fc_worker_bind_sources(
        &self,
        session_db: &GatewaySessionDb,
        proj_id: i64,
        worker_id: &str,
    ) -> Result<(), String> {
        self.ensure_fc_proj_nas_roots(proj_id).await?;
        self.materialize_proj_workspace(session_db, proj_id).await?;
        self.ensure_worker_root(proj_id, worker_id).await?;
        Ok(())
    }
}
