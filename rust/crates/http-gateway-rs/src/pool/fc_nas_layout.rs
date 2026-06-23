//! Gateway-side NAS host path layout (logical rel → local filesystem). Author: kejiqing
//!
//! FC mode: Gateway prepares worker dirs and session symlinks on NAS; e2b binds
//! `proj_N/workers/{workerId}` into sandboxes at create.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use claw_fc_sandbox_client::{
    proj_home_rel, session_rel, session_symlink_target, sessions_root_rel, tap_traces_rel,
    worker_rel, workers_root_rel,
};

use crate::session_db::GatewaySessionDb;

static WORKER_ID_SEQ: AtomicU64 = AtomicU64::new(1);

/// Pre-allocate NAS worker directory name before e2b sandbox create (bind is static).
#[must_use]
pub fn allocate_worker_id() -> String {
    let seq = WORKER_ID_SEQ.fetch_add(1, Ordering::Relaxed);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("wrk_{ms:x}_{seq:x}")
}

/// True when gateway process can mkdir/symlink on the same NAS tree e2b binds.
#[must_use]
pub fn fc_nas_layout_active(nas_root: &Path) -> bool {
    if let Ok(m) = std::env::var("CLAW_NAS_HOST_MOUNT") {
        let trimmed = m.trim();
        if !trimmed.is_empty() {
            if Path::new(trimmed).exists() {
                return true;
            }
            // Gateway container: CLAW_NAS_HOST_MOUNT names the e2b Mac host path (/Volumes/…)
            // while NAS is bind-mounted at work_root (nas_root). Still prepare dirs there. kejiqing
            return nas_root.is_dir();
        }
    }
    // Workspace volume direct-bind without explicit host mount env.
    nas_root.join("proj_1").exists()
}

/// Resolved NAS host root for Gateway file operations (`mkdir`, symlink, chown).
#[must_use]
pub fn nas_host_root(work_root: &Path, _pool_rpc_host_work_root: Option<&Path>) -> PathBuf {
    if let Ok(m) = std::env::var("CLAW_NAS_HOST_MOUNT") {
        let trimmed = m.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.exists() {
                return p;
            }
        }
    }
    work_root.to_path_buf()
}

/// `{nas_root}/proj_{id}/home`
#[must_use]
pub fn proj_home_host_path(nas_root: &Path, proj_id: i64) -> PathBuf {
    nas_root.join(proj_home_rel(proj_id))
}

/// `{nas_root}/proj_{id}/workers/{worker_id}`
#[must_use]
pub fn worker_host_path(nas_root: &Path, proj_id: i64, worker_id: &str) -> PathBuf {
    nas_root.join(worker_rel(proj_id, worker_id))
}

/// `{nas_root}/proj_{id}/sessions/{segment}`
#[must_use]
pub fn session_host_path(nas_root: &Path, proj_id: i64, session_segment: &str) -> PathBuf {
    nas_root.join(session_rel(proj_id, session_segment))
}

/// Ensure `proj_N/sessions` exists (symlink namespace).
pub async fn ensure_proj_sessions_root_on_nas(
    nas_root: &Path,
    proj_id: i64,
) -> Result<PathBuf, String> {
    let sessions_abs = nas_root.join(sessions_root_rel(proj_id));
    tokio::fs::create_dir_all(&sessions_abs)
        .await
        .map_err(|e| format!("mkdir NAS sessions root {}: {e}", sessions_abs.display()))?;
    Ok(sessions_abs)
}

/// Ensure `proj_N/workers` exists.
pub async fn ensure_proj_workers_root_on_nas(
    nas_root: &Path,
    proj_id: i64,
) -> Result<PathBuf, String> {
    let workers_abs = nas_root.join(workers_root_rel(proj_id));
    tokio::fs::create_dir_all(&workers_abs)
        .await
        .map_err(|e| format!("mkdir NAS workers root {}: {e}", workers_abs.display()))?;
    Ok(workers_abs)
}

/// Ensure worker NAS root exists before e2b static bind.
pub async fn ensure_worker_root_on_nas(
    runtime_bin: &str,
    nas_root: &Path,
    proj_id: i64,
    worker_id: &str,
) -> Result<PathBuf, String> {
    ensure_proj_workers_root_on_nas(nas_root, proj_id).await?;
    let worker_abs = worker_host_path(nas_root, proj_id, worker_id);
    tokio::fs::create_dir_all(worker_abs.join(".claw"))
        .await
        .map_err(|e| format!("mkdir NAS worker root {}: {e}", worker_abs.display()))?;
    super::session_mount_ownership::ensure_session_tree_owned_for_worker_with_runtime_fallback(
        runtime_bin,
        &worker_abs,
    )
    .await?;
    Ok(worker_abs)
}

/// Replace any prior session path (legacy real dir or old symlink) before linking to worker.
async fn replace_session_link_path(link_path: &Path) -> Result<(), String> {
    let meta = match tokio::fs::symlink_metadata(link_path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("stat {}: {e}", link_path.display())),
    };
    if meta.file_type().is_symlink() {
        tokio::fs::remove_file(link_path)
            .await
            .map_err(|e| format!("rm symlink {}: {e}", link_path.display()))?;
        return Ok(());
    }
    if meta.is_dir() {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let backup = link_path.with_extension(format!("legacy-{ms}"));
        tokio::fs::rename(link_path, &backup).await.map_err(|e| {
            format!(
                "rename legacy session dir {} -> {}: {e}",
                link_path.display(),
                backup.display()
            )
        })?;
        return Ok(());
    }
    tokio::fs::remove_file(link_path)
        .await
        .map_err(|e| format!("rm {}: {e}", link_path.display()))
}

/// `sessions/{segment}` → `../workers/{worker_id}` (Gateway routing).
pub async fn link_session_to_worker(
    nas_root: &Path,
    proj_id: i64,
    session_segment: &str,
    worker_id: &str,
) -> Result<PathBuf, String> {
    ensure_proj_sessions_root_on_nas(nas_root, proj_id).await?;
    let link_path = session_host_path(nas_root, proj_id, session_segment);
    replace_session_link_path(&link_path).await?;
    let target = session_symlink_target(worker_id);
    #[cfg(unix)]
    {
        tokio::fs::symlink(&target, &link_path)
            .await
            .map_err(|e| format!("symlink {} -> {}: {e}", link_path.display(), target))?;
    }
    #[cfg(not(unix))]
    {
        return Err("NAS session symlink requires unix".into());
    }
    Ok(link_path)
}

/// Remove session symlink after terminal/stop (keeps worker dir for warm reuse).
pub async fn unlink_session_symlink(
    nas_root: &Path,
    proj_id: i64,
    session_segment: &str,
) -> Result<(), String> {
    let link_path = session_host_path(nas_root, proj_id, session_segment);
    if link_path.exists() || link_path.is_symlink() {
        remove_path_all(&link_path).await?;
    }
    Ok(())
}

/// `{nas_root}/tap-traces` (shared claude-tap traces bind source).
pub async fn ensure_tap_traces_root_on_nas(nas_root: &Path) -> Result<PathBuf, String> {
    let traces_abs = nas_root.join(tap_traces_rel());
    tokio::fs::create_dir_all(&traces_abs)
        .await
        .map_err(|e| format!("mkdir NAS tap-traces {}: {e}", traces_abs.display()))?;
    Ok(traces_abs)
}

/// `proj_N/home` must exist on the e2b bind source before `/claw_ds` attach.
pub async fn ensure_proj_home_dir_on_nas(nas_root: &Path, proj_id: i64) -> Result<PathBuf, String> {
    let home_abs = proj_home_host_path(nas_root, proj_id);
    tokio::fs::create_dir_all(&home_abs)
        .await
        .map_err(|e| format!("mkdir NAS proj home {}: {e}", home_abs.display()))?;
    Ok(home_abs)
}

/// FC terminal/solve prep: sessions, workers, proj home, tap-traces on NAS bind source.
pub async fn ensure_fc_proj_nas_roots(nas_root: &Path, proj_id: i64) -> Result<(), String> {
    ensure_proj_sessions_root_on_nas(nas_root, proj_id).await?;
    ensure_proj_workers_root_on_nas(nas_root, proj_id).await?;
    ensure_proj_home_dir_on_nas(nas_root, proj_id).await?;
    ensure_tap_traces_root_on_nas(nas_root).await?;
    Ok(())
}

/// PG project config + worker dir on NAS before e2b `nasConfig` bind at create.
pub async fn prepare_fc_worker_bind_sources(
    session_db: &GatewaySessionDb,
    runtime_bin: &str,
    nas_root: &Path,
    proj_id: i64,
    worker_id: &str,
) -> Result<(), String> {
    ensure_fc_proj_nas_roots(nas_root, proj_id).await?;
    crate::session_terminal_api::materialize_ovs_proj_workspace(session_db, nas_root, proj_id)
        .await
        .map_err(|e| format!("materialize proj_{proj_id}/home on NAS: {e}"))?;
    ensure_worker_root_on_nas(runtime_bin, nas_root, proj_id, worker_id).await?;
    Ok(())
}

async fn remove_path_all(path: &Path) -> Result<(), String> {
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| format!("stat {}: {e}", path.display()))?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        tokio::fs::remove_dir_all(path)
            .await
            .map_err(|e| format!("rmdir {}: {e}", path.display()))?;
    } else {
        tokio::fs::remove_file(path)
            .await
            .map_err(|e| format!("rm {}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_host_path_format() {
        let root = PathBuf::from("/mnt/nas0");
        let p = session_host_path(&root, 1, "ovs-1");
        assert_eq!(p, PathBuf::from("/mnt/nas0/proj_1/sessions/ovs-1"));
    }

    #[test]
    fn worker_host_path_format() {
        let root = PathBuf::from("/mnt/nas0");
        let p = worker_host_path(&root, 1, "wrk_abc");
        assert_eq!(p, PathBuf::from("/mnt/nas0/proj_1/workers/wrk_abc"));
    }

    #[test]
    fn allocate_worker_id_unique() {
        let a = allocate_worker_id();
        let b = allocate_worker_id();
        assert!(a.starts_with("wrk_"));
        assert_ne!(a, b);
    }

    #[test]
    fn fc_nas_layout_active_dir_when_host_mount_env_only() {
        let dir = std::env::temp_dir().join(format!("claw_fc_nas_layout_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let prev = std::env::var("CLAW_NAS_HOST_MOUNT").ok();
        std::env::set_var("CLAW_NAS_HOST_MOUNT", "/Volumes/claw-nas-no-such-path");
        assert!(fc_nas_layout_active(&dir));
        match prev {
            Some(v) => std::env::set_var("CLAW_NAS_HOST_MOUNT", v),
            None => std::env::remove_var("CLAW_NAS_HOST_MOUNT"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
