//! NAS logical paths and guest mount points (Claw contract). Author: kejiqing
//!
//! Gateway pool maps **logical rel paths** under NAS export root to guest mount dirs.
//! e2b binds `{hostMountRoot}/{relPath}` → `mountDir` at sandbox create (static).
//! Each worker binds `proj_N/workers/{workerId}` → `/claw_host_root`; Gateway links
//! `proj_N/sessions/{session}` → `../workers/{workerId}` at terminal/start.

/// OVS / observe singleton: NAS export root inside sandbox.
pub const GUEST_CLAW_WS: &str = "/claw_ws";
/// Project home (warm bake, OVS REPL cwd).
pub const GUEST_CLAW_DS: &str = "/claw_ds";
/// Worker session workspace (`proj_N/workers/{workerId}` bind target).
pub const GUEST_CLAW_HOST_ROOT: &str = "/claw_host_root";

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

/// Worker / observe guest mount for shared NAS `tap-traces/` (proxy writes + Live reads).
pub const GUEST_CLAW_TAP_TRACES: &str = "/claw_tap_traces";

/// `proj_N/sessions` → `/claw_sessions` (OVS interactive jsonl; FC warm worker).
pub const GUEST_CLAW_SESSIONS: &str = "/claw_sessions";

/// `proj_N/home` → `/claw_ds`.
#[must_use]
pub fn proj_home_rel(proj_id: i64) -> String {
    format!("proj_{proj_id}/home")
}

/// `proj_N/sessions` (symlink namespace; not bind-mounted into workers).
#[must_use]
pub fn sessions_root_rel(proj_id: i64) -> String {
    format!("proj_{proj_id}/sessions")
}

/// `proj_N/workers` (per-worker NAS roots).
#[must_use]
pub fn workers_root_rel(proj_id: i64) -> String {
    format!("proj_{proj_id}/workers")
}

/// `proj_N/workers/{worker_id}` → `/claw_host_root`.
#[must_use]
pub fn worker_rel(proj_id: i64, worker_id: &str) -> String {
    format!("proj_{proj_id}/workers/{worker_id}")
}

/// Logical path under NAS export: `proj_N/sessions/{segment}` (symlink to worker).
#[must_use]
pub fn session_rel(proj_id: i64, session_segment: &str) -> String {
    format!("proj_{proj_id}/sessions/{session_segment}")
}

/// Relative symlink target from `sessions/{segment}` to worker dir.
#[must_use]
pub fn session_symlink_target(worker_id: &str) -> String {
    format!("../workers/{worker_id}")
}

/// Active session workspace inside sandbox (flat worker root).
#[must_use]
pub fn guest_worker_work_dir() -> &'static str {
    GUEST_CLAW_HOST_ROOT
}

/// Warm worker: project home + worker root + sessions + shared tap-traces (static bind at create).
#[must_use]
pub fn warm_worker_mounts(proj_id: i64, worker_id: &str) -> Vec<(String, String)> {
    vec![
        (proj_home_rel(proj_id), GUEST_CLAW_DS.into()),
        (worker_rel(proj_id, worker_id), GUEST_CLAW_HOST_ROOT.into()),
        (sessions_root_rel(proj_id), GUEST_CLAW_SESSIONS.into()),
        (tap_traces_rel().to_string(), GUEST_CLAW_TAP_TRACES.into()),
    ]
}

/// OVS / observe singleton: export root only.
#[must_use]
pub fn ovs_root_mounts() -> Vec<(String, String)> {
    vec![(export_root_rel(), GUEST_CLAW_WS.into())]
}

/// Cold one-shot worker: worker root + optional project home (OVS REPL) + shared tap-traces.
#[must_use]
pub fn worker_mounts(
    proj_id: i64,
    worker_id: &str,
    include_proj_home: bool,
) -> Vec<(String, String)> {
    let mut mounts = vec![
        (worker_rel(proj_id, worker_id), GUEST_CLAW_HOST_ROOT.into()),
        (tap_traces_rel().to_string(), GUEST_CLAW_TAP_TRACES.into()),
    ];
    if include_proj_home {
        mounts.push((proj_home_rel(proj_id), GUEST_CLAW_DS.into()));
    }
    mounts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_paths() {
        assert_eq!(proj_home_rel(2), "proj_2/home");
        assert_eq!(workers_root_rel(1), "proj_1/workers");
        assert_eq!(worker_rel(1, "wrk_abc"), "proj_1/workers/wrk_abc");
        assert_eq!(session_rel(1, "ovs-1"), "proj_1/sessions/ovs-1");
        assert_eq!(session_symlink_target("wrk_abc"), "../workers/wrk_abc");
        assert_eq!(guest_worker_work_dir(), "/claw_host_root");
    }

    #[test]
    fn warm_mounts_worker_root() {
        let warm = warm_worker_mounts(1, "wrk_1");
        assert_eq!(warm.len(), 4);
        assert_eq!(warm[1].0, "proj_1/workers/wrk_1");
        assert_eq!(warm[1].1, GUEST_CLAW_HOST_ROOT);
        assert_eq!(warm[2].0, "proj_1/sessions");
        assert_eq!(warm[2].1, GUEST_CLAW_SESSIONS);
        assert_eq!(warm[3].0, "tap-traces");
        assert_eq!(warm[3].1, GUEST_CLAW_TAP_TRACES);
    }
}
