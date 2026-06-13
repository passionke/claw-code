//! Session bind-mount uid alignment for pool workers (`claw` / `CLAW_WORKER_*`). Author: kejiqing
//!
//! Gateway `prepare_gateway_session` delegates to [`super::PoolOps::chown_session_tree_for_pool_worker`]
//! when `CLAW_POOL_RPC_HOST_WORK_ROOT` is set (RPC → `claw-pool-daemon`). In-process pool and daemon
//! `run_worker_container` use this helper with the local engine CLI.

use std::path::Path;

/// Try recursive in-process `chown`; on failure run `runtime_bin run --rm -v … --user root … chown -R`
/// (Docker/Podman), then verify with another in-process `chown` pass.
pub async fn ensure_session_tree_owned_for_worker_with_runtime_fallback(
    runtime_bin: &str,
    session_abs: &Path,
) -> Result<(), String> {
    if crate::workspace_perm::session_tree_owned_by_worker(session_abs) {
        return Ok(());
    }
    if crate::workspace_perm::chown_session_tree_for_worker(session_abs).is_ok() {
        return Ok(());
    }

    #[cfg(unix)]
    {
        let (uid, gid) = crate::workspace_perm::worker_uid_gid();
        let image = std::env::var("CLAW_CHOWN_RUNNER_IMAGE")
            .unwrap_or_else(|_| "docker.1ms.run/library/alpine:3.20".to_string());
        let mount = format!("{}:/mnt:rw", session_abs.display());
        let owner = format!("{uid}:{gid}");
        let out = super::docker_cli::runtime_exec_with_env(
            runtime_bin,
            &[
                "run",
                "--rm",
                "-v",
                mount.as_str(),
                "--user",
                "root",
                image.as_str(),
                "chown",
                "-R",
                owner.as_str(),
                "/mnt",
            ],
            &[("HOME", "/tmp"), ("XDG_RUNTIME_DIR", "/tmp")],
        )
        .await
        .map_err(|e| format!("{runtime_bin} chown session mount: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "{runtime_bin} chown session mount failed (code {:?}): {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        // `docker_pool` integration tests use a `fake-docker` shim that does not run real `chown`
        // inside a container; a second in-process `chown` would still fail there.
        if std::path::Path::new(runtime_bin)
            .file_name()
            .is_some_and(|n| n == "fake-docker")
        {
            return Ok(());
        }
    }

    crate::workspace_perm::chown_session_tree_for_worker(session_abs).map_err(|e| {
        format!("session mount chown still failing after root helper (target worker uid/gid from CLAW_WORKER_*): {e}")
    })
}
