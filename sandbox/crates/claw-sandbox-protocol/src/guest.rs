//! Sandbox guest volume + exec actor contract (RPC boundary). Author: kejiqing

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::isolation::IsolationMode;
use crate::session::{DS_MOUNT_TARGET, GUEST_WORK_ROOT};

/// Worker mount semantics — Gateway must not pass raw absolute paths on guest I/O RPCs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestVolume {
    /// `/claw_host_root` — per-turn session workspace (task, jsonl, tar, reports).
    SessionWorkspace,
    /// `/claw_ds` — Admin project config materialized from PG (lock after materialize).
    ProjectConfig,
}

impl GuestVolume {
    #[must_use]
    pub fn mount_path(self) -> &'static str {
        match self {
            Self::SessionWorkspace => GUEST_WORK_ROOT,
            Self::ProjectConfig => DS_MOUNT_TARGET,
        }
    }
}

/// Who runs `guest_exec_sh` — pool root for wipe/lock; slot worker for session scripts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestExecActor {
    /// `claw` (or configured pool exec user) for all solve/materialize work. Author: kejiqing
    #[default]
    SlotWorker,
    /// Pool host root (`0:0`) — wipe tmpfs, lock `/claw_ds`; Gateway must not use for solve.
    PoolRoot,
}

/// Resolved exec identity returned on `acquire` (audit / Admin).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotExecIdentity {
    pub isolation: IsolationMode,
    /// `podman exec --user` value for this lease (`claw` or `uid:gid`).
    pub exec_user: String,
}

/// Normalize `rel_path` (no `..`, no absolute); join under [`GuestVolume::mount_path`].
pub fn resolve_guest_path(volume: GuestVolume, rel_path: &str) -> Result<String, String> {
    let rel = normalize_rel_path(rel_path)?;
    let joined = if rel.as_os_str().is_empty() {
        volume.mount_path().to_string()
    } else {
        format!("{}/{}", volume.mount_path(), rel.to_string_lossy())
    };
    Ok(joined)
}

/// Validate absolute guest path is under an allowed volume root.
pub fn validate_guest_abs_path(path: &str) -> Result<(GuestVolume, PathBuf), String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("guest path must be non-empty".into());
    }
    let abs = Path::new(trimmed);
    if abs.starts_with(DS_MOUNT_TARGET) {
        let rel = abs
            .strip_prefix(DS_MOUNT_TARGET)
            .map(|p| p.strip_prefix("/").unwrap_or(p))
            .unwrap_or(Path::new(""));
        return Ok((GuestVolume::ProjectConfig, rel.to_path_buf()));
    }
    if abs.starts_with(GUEST_WORK_ROOT) {
        let rel = abs
            .strip_prefix(GUEST_WORK_ROOT)
            .map(|p| p.strip_prefix("/").unwrap_or(p))
            .unwrap_or(Path::new(""));
        return Ok((GuestVolume::SessionWorkspace, rel.to_path_buf()));
    }
    Err(format!(
        "guest path must be under {GUEST_WORK_ROOT} or {DS_MOUNT_TARGET}, got {trimmed}"
    ))
}

fn normalize_rel_path(rel_path: &str) -> Result<PathBuf, String> {
    let trimmed = rel_path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(PathBuf::new());
    }
    let path = Path::new(trimmed);
    for comp in path.components() {
        match comp {
            Component::Normal(_) => {}
            Component::CurDir => {}
            _ => return Err(format!("invalid rel_path {rel_path:?}")),
        }
    }
    Ok(path.to_path_buf())
}

/// Sandbox runs this as slot worker after session tar extract.
pub const GUEST_PREPARE_SESSION_WORKSPACE_SH: &str = r#"set -eu
root='/claw_host_root'
rm -rf "$root/home"
if [ -L "$root/.claw/skills" ]; then
  rm -f "$root/.claw/skills"
fi
"#;

/// Wipe `/claw_ds` project-config tmpfs (pool root). Author: kejiqing
pub const GUEST_WIPE_DS_SH: &str = r#"set -eu
find /claw_ds -mindepth 1 -delete 2>/dev/null || true
"#;

/// Wipe `/claw_host_root` session workspace (slot worker user — root cannot unlink claw-owned files in userns strict workers). Author: kejiqing
pub const GUEST_WIPE_WORK_ROOT_SH: &str = r#"set -eu
find /claw_host_root -mindepth 1 -exec rm -rf {} + 2>/dev/null || true
"#;

/// Legacy combined wipe; prefer [`GUEST_WIPE_DS_SH`] + [`GUEST_WIPE_WORK_ROOT_SH`] as separate exec users. Author: kejiqing
pub const GUEST_WIPE_EPHEMERAL_MOUNTS_SH: &str = concat!(
    "set -eu\n",
    "find /claw_ds -mindepth 1 -delete 2>/dev/null || true\n",
    "find /claw_host_root -mindepth 1 -exec rm -rf {} + 2>/dev/null || true\n",
);

/// Pool root after `/claw_ds` guest_write: chmod ro for worker user (`claw`). Author: kejiqing
pub const GUEST_LOCK_PROJECT_CONFIG_SH: &str = r#"set -eu
ds='/claw_ds'
if [ ! -d "$ds" ]; then exit 0; fi
find "$ds" -type f -exec chmod a-w,a+r {} + 2>/dev/null || true
find "$ds" -type d -exec chmod a-w,a+rx {} + 2>/dev/null || true
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_session_path() {
        assert_eq!(
            resolve_guest_path(GuestVolume::SessionWorkspace, "gateway-solve-task.json").unwrap(),
            "/claw_host_root/gateway-solve-task.json"
        );
    }

    #[test]
    fn resolve_project_config_path() {
        assert_eq!(
            resolve_guest_path(GuestVolume::ProjectConfig, ".claw/settings.json").unwrap(),
            "/claw_ds/.claw/settings.json"
        );
    }

    #[test]
    fn reject_parent_rel() {
        assert!(resolve_guest_path(GuestVolume::SessionWorkspace, "../etc/passwd").is_err());
    }

    #[test]
    fn validate_abs_under_ds() {
        let (vol, rel) = validate_guest_abs_path("/claw_ds/CLAUDE.md").unwrap();
        assert_eq!(vol, GuestVolume::ProjectConfig);
        assert_eq!(rel, Path::new("CLAUDE.md"));
    }
}
