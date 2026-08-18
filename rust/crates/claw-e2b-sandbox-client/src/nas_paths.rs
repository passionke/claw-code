//! NAS logical paths and guest mount points (Claw contract). Author: kejiqing
//!
//! Gateway pool maps **logical rel paths** under NAS export root to guest mount dirs.
//! Layout: `{clusterId}/proj_{N}/home|sessions|workers/...`.
//! Sessions are real directories (context `SoT`); workers are execution cache only.

/// OVS / observe singleton: NAS export root inside sandbox.
pub const GUEST_CLAW_WS: &str = "/claw_ws";
/// Project home / `ds_home` (readonly in worker sandboxes).
pub const GUEST_CLAW_DS: &str = "/claw_ds";
/// Worker execution cache (`proj_N/workers/{workerId}` bind target).
pub const GUEST_CLAW_HOST_ROOT: &str = "/claw_host_root";
/// Session namespace (`proj_N/sessions` bind target).
pub const GUEST_CLAW_SESSIONS: &str = "/claw_sessions";

/// e2b `nasConfig.mountPoints[]` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NasMountPoint {
    pub rel_path: String,
    pub mount_dir: String,
    pub read_only: bool,
}

/// NAS export root (empty rel) → `/claw_ws`.
#[must_use]
pub fn export_root_rel() -> String {
    String::new()
}

/// Shared claude-tap trace dir on NAS export (`tap-traces/`).
#[must_use]
pub fn tap_traces_rel() -> &'static str {
    "tap-traces"
}

/// Worker / observe guest mount for shared NAS `tap-traces/`.
pub const GUEST_CLAW_TAP_TRACES: &str = "/claw_tap_traces";

/// `{clusterId}/proj_N/home` → `/claw_ds`.
#[must_use]
pub fn proj_home_rel(cluster_id: &str, proj_id: i64) -> String {
    format!("{cluster_id}/proj_{proj_id}/home")
}

/// Worker path after binding project home at `/claw_ds`. `rel_under_home` is e.g. `<destRel>/README`. Author: kejiqing
#[must_use]
pub fn guest_path_under_claw_ds(rel_under_home: &str) -> String {
    let rel = rel_under_home.trim_start_matches('/');
    if rel.is_empty() {
        GUEST_CLAW_DS.to_string()
    } else {
        format!("{GUEST_CLAW_DS}/{rel}")
    }
}

/// Map a NAS export-rel file/dir under project home to the worker guest path. Author: kejiqing
#[must_use]
pub fn guest_path_from_nas_proj_rel(cluster_id: &str, proj_id: i64, nas_rel: &str) -> Option<String> {
    let home = proj_home_rel(cluster_id, proj_id);
    let nas = nas_rel.trim_start_matches('/');
    if nas == home {
        return Some(GUEST_CLAW_DS.to_string());
    }
    let prefix = format!("{home}/");
    let rest = nas.strip_prefix(&prefix)?;
    Some(guest_path_under_claw_ds(rest))
}

/// `{clusterId}/proj_N/sessions`.
#[must_use]
pub fn sessions_root_rel(cluster_id: &str, proj_id: i64) -> String {
    format!("{cluster_id}/proj_{proj_id}/sessions")
}

/// `{clusterId}/proj_N/workers`.
#[must_use]
pub fn workers_root_rel(cluster_id: &str, proj_id: i64) -> String {
    format!("{cluster_id}/proj_{proj_id}/workers")
}

/// `{clusterId}/proj_N/workers/{worker_id}` → `/claw_host_root`.
#[must_use]
pub fn worker_rel(cluster_id: &str, proj_id: i64, worker_id: &str) -> String {
    format!("{cluster_id}/proj_{proj_id}/workers/{worker_id}")
}

/// `{clusterId}/proj_N/sessions/{segment}` — real session directory.
#[must_use]
pub fn session_rel(cluster_id: &str, proj_id: i64, session_segment: &str) -> String {
    format!("{cluster_id}/proj_{proj_id}/sessions/{session_segment}")
}

/// Relative symlink target from `sessions/{segment}/ds` to project home (readonly).
#[must_use]
pub fn session_ds_symlink_target() -> &'static str {
    "../../home"
}

/// Legacy: relative symlink from `sessions/{segment}` to worker dir (deprecated).
#[must_use]
pub fn session_symlink_target(worker_id: &str) -> String {
    format!("../workers/{worker_id}")
}

/// Guest path for a session workspace root.
#[must_use]
pub fn guest_session_root(session_segment: &str) -> String {
    format!("{GUEST_CLAW_SESSIONS}/{session_segment}")
}

/// Active session workspace inside sandbox (per-turn cwd).
#[must_use]
pub fn guest_session_work_dir(session_segment: &str) -> String {
    guest_session_root(session_segment)
}

/// Deprecated: flat worker root was incorrectly used as solve cwd.
#[must_use]
pub fn guest_worker_work_dir() -> &'static str {
    GUEST_CLAW_HOST_ROOT
}

/// OVS workspace folder inside relaxed worker (same guest path as project-config root).
#[must_use]
pub fn ovs_workspace_folder() -> &'static str {
    GUEST_CLAW_DS
}

/// Browser `?folder=` query for relaxed worker built-in OVS.
#[must_use]
pub fn ovs_folder_url(ovs_base_url: &str) -> String {
    format!(
        "{}?folder={}",
        ovs_base_url.trim_end_matches('/'),
        ovs_workspace_folder()
    )
}

/// Warm worker: home (ro in strict, rw in relaxed) + sessions + worker cache.
#[must_use]
pub fn warm_worker_mounts(
    cluster_id: &str,
    proj_id: i64,
    worker_id: &str,
    proj_home_read_only: bool,
) -> Vec<NasMountPoint> {
    vec![
        NasMountPoint {
            rel_path: proj_home_rel(cluster_id, proj_id),
            mount_dir: GUEST_CLAW_DS.into(),
            read_only: proj_home_read_only,
        },
        NasMountPoint {
            rel_path: sessions_root_rel(cluster_id, proj_id),
            mount_dir: GUEST_CLAW_SESSIONS.into(),
            read_only: false,
        },
        NasMountPoint {
            rel_path: worker_rel(cluster_id, proj_id, worker_id),
            mount_dir: GUEST_CLAW_HOST_ROOT.into(),
            read_only: false,
        },
    ]
}

/// OVS / observe singleton: export root only.
#[allow(dead_code)]
#[must_use]
pub fn ovs_root_mounts() -> Vec<NasMountPoint> {
    vec![NasMountPoint {
        rel_path: export_root_rel(),
        mount_dir: GUEST_CLAW_WS.into(),
        read_only: false,
    }]
}

/// Cold one-shot worker mounts.
#[must_use]
pub fn worker_mounts(
    cluster_id: &str,
    proj_id: i64,
    worker_id: &str,
    include_proj_home: bool,
) -> Vec<NasMountPoint> {
    let mut mounts = vec![
        NasMountPoint {
            rel_path: sessions_root_rel(cluster_id, proj_id),
            mount_dir: GUEST_CLAW_SESSIONS.into(),
            read_only: false,
        },
        NasMountPoint {
            rel_path: worker_rel(cluster_id, proj_id, worker_id),
            mount_dir: GUEST_CLAW_HOST_ROOT.into(),
            read_only: false,
        },
    ];
    if include_proj_home {
        mounts.insert(
            0,
            NasMountPoint {
                rel_path: proj_home_rel(cluster_id, proj_id),
                mount_dir: GUEST_CLAW_DS.into(),
                read_only: true,
            },
        );
    }
    mounts
}

#[cfg(test)]
mod tests {
    use super::*;

    const CID: &str = "dev-stable";

    #[test]
    fn rel_paths_with_cluster() {
        assert_eq!(proj_home_rel(CID, 2), "dev-stable/proj_2/home");
        assert_eq!(workers_root_rel(CID, 1), "dev-stable/proj_1/workers");
        assert_eq!(
            worker_rel(CID, 1, "wrk_abc"),
            "dev-stable/proj_1/workers/wrk_abc"
        );
        assert_eq!(
            session_rel(CID, 1, "ovs-1"),
            "dev-stable/proj_1/sessions/ovs-1"
        );
        assert_eq!(session_ds_symlink_target(), "../../home");
    }

    #[test]
    fn guest_session_root_format() {
        assert_eq!(guest_session_root("seg-a"), "/claw_sessions/seg-a");
    }

    #[test]
    fn ovs_workspace_folder_is_claw_ds() {
        assert_eq!(ovs_workspace_folder(), "/claw_ds");
        assert!(ovs_folder_url("http://3000-sbx_abc.supone.top/ovs").contains("folder=/claw_ds"));
        assert!(!ovs_folder_url("http://3000-sbx_abc.supone.top/ovs").contains("/claw_ws/"));
    }

    #[test]
    fn warm_mounts_include_sessions_and_ro_home() {
        let warm = warm_worker_mounts(CID, 1, "wrk_1", true);
        assert_eq!(warm.len(), 3);
        assert!(warm[0].read_only);
        assert_eq!(warm[0].mount_dir, GUEST_CLAW_DS);
        assert_eq!(warm[1].mount_dir, GUEST_CLAW_SESSIONS);
        assert_eq!(warm[2].mount_dir, GUEST_CLAW_HOST_ROOT);
        let relaxed = warm_worker_mounts(CID, 1, "wrk_1", false);
        assert!(!relaxed[0].read_only);
    }

    #[test]
    fn git_import_nas_rel_is_under_home_mount_and_maps_to_claw_ds() {
        let dest = "Hello-World";
        let file = "README";
        let nas_dest = format!("{}/{dest}", proj_home_rel(CID, 12));
        let nas_file = format!("{nas_dest}/{file}");
        let mounts = warm_worker_mounts(CID, 12, "wrk_1", true);
        let home = &mounts[0];
        assert_eq!(home.rel_path, proj_home_rel(CID, 12));
        assert_eq!(home.mount_dir, GUEST_CLAW_DS);
        assert!(
            nas_file.starts_with(&format!("{}/", home.rel_path)),
            "git dest must sit inside the /claw_ds bind source so the worker can read it"
        );
        assert_eq!(
            guest_path_from_nas_proj_rel(CID, 12, &nas_dest).as_deref(),
            Some("/claw_ds/Hello-World")
        );
        assert_eq!(
            guest_path_from_nas_proj_rel(CID, 12, &nas_file).as_deref(),
            Some("/claw_ds/Hello-World/README")
        );
        assert!(home.read_only, "strict worker still reads /claw_ds (ro bind)");
        assert!(guest_path_from_nas_proj_rel(CID, 12, "proj_12/home/Hello-World").is_none());
        assert!(guest_path_from_nas_proj_rel(CID, 12, &format!("{CID}/proj_12/workers/w")).is_none());
    }

    #[test]
    fn two_git_dests_are_siblings_under_claw_ds_not_flattened() {
        let a = guest_path_from_nas_proj_rel(CID, 1, &format!("{}/a/README", proj_home_rel(CID, 1)));
        let b = guest_path_from_nas_proj_rel(CID, 1, &format!("{}/b/README", proj_home_rel(CID, 1)));
        assert_eq!(a.as_deref(), Some("/claw_ds/a/README"));
        assert_eq!(b.as_deref(), Some("/claw_ds/b/README"));
        assert_ne!(a, b);
    }
}
