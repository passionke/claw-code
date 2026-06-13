//! Mechanical guest file I/O via container exec (tee, cat, tar). Author: kejiqing

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use claw_sandbox_protocol::{
    GuestFileBytes, DS_MOUNT_TARGET, GUEST_WIPE_DS_SH, GUEST_WIPE_WORK_ROOT_SH, GUEST_WORK_ROOT,
};

use crate::runtime::docker_cli::{runtime_exec, runtime_exec_stdin};

/// Wipe ephemeral tmpfs before materialize: `/claw_ds` as root, `/claw_host_root` as worker user. Author: kejiqing
pub async fn wipe_guest_ephemeral_mounts(
    runtime_bin: &str,
    container_name: &str,
    worker_exec_user: &str,
) -> Result<(), String> {
    exec_sh_lc_as_user(runtime_bin, container_name, "0:0", GUEST_WIPE_DS_SH).await?;
    exec_sh_lc_as_user(
        runtime_bin,
        container_name,
        worker_exec_user,
        GUEST_WIPE_WORK_ROOT_SH,
    )
    .await
}

/// Legacy alias — prefer [`wipe_guest_ephemeral_mounts`].
pub async fn wipe_guest_work_root(
    runtime_bin: &str,
    container_name: &str,
    worker_exec_user: &str,
) -> Result<(), String> {
    wipe_guest_ephemeral_mounts(runtime_bin, container_name, worker_exec_user).await
}

pub async fn write_file_via_exec_user(
    runtime_bin: &str,
    container: &str,
    worker_exec_user: &str,
    dest_path: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let mkdir_script = format!("mkdir -p \"$(dirname '{dest_path}')\"");
    exec_sh_lc_as_user(runtime_bin, container, worker_exec_user, &mkdir_script).await?;
    let argv = [
        "exec",
        "-i",
        "--user",
        worker_exec_user,
        container,
        "tee",
        dest_path,
    ];
    let out = runtime_exec_stdin(runtime_bin, &argv, bytes)
        .await
        .map_err(|e| format!("{runtime_bin} exec tee: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exec tee {} failed: {}",
            runtime_bin,
            dest_path,
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

pub async fn read_files_base64(
    runtime_bin: &str,
    container: &str,
    paths: &[String],
) -> Result<Vec<GuestFileBytes>, String> {
    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let script = format!("cat '{path}' 2>/dev/null || true");
        let body = exec_sh_lc_capture(runtime_bin, container, &script).await?;
        out.push(GuestFileBytes {
            path: path.clone(),
            bytes_b64: B64.encode(body.as_bytes()),
        });
    }
    Ok(out)
}

/// Extract base64 tar.gz under `guest_path_prefix` inside [`GUEST_WORK_ROOT`]. Author: kejiqing
pub async fn extract_tar_b64_under_prefix(
    runtime_bin: &str,
    container_name: &str,
    worker_exec_user: &str,
    guest_path_prefix: &str,
    tar_b64: &str,
) -> Result<(), String> {
    let trimmed = guest_path_prefix.trim();
    let dest_root = if trimmed == GUEST_WORK_ROOT
        || trimmed.starts_with(&format!("{GUEST_WORK_ROOT}/"))
    {
        trimmed.to_string()
    } else if trimmed == DS_MOUNT_TARGET || trimmed.starts_with(&format!("{DS_MOUNT_TARGET}/")) {
        trimmed.to_string()
    } else {
        let prefix = trimmed.trim_start_matches('/');
        if prefix.is_empty() {
            GUEST_WORK_ROOT.to_string()
        } else {
            format!("{GUEST_WORK_ROOT}/{prefix}")
        }
    };
    let script = format!(
        r#"set -eu
ws_tmp=$(mktemp -d)
staging="$ws_tmp/staging"
mkdir -p "$staging" "{dest_root}"
trap 'rm -rf "$ws_tmp"' EXIT
base64 -d > "$ws_tmp/archive.tar.gz"
tar -xzf "$ws_tmp/archive.tar.gz" -C "$staging" -m --no-same-owner --no-same-permissions 2>/dev/null \
  || tar -xzf "$ws_tmp/archive.tar.gz" -C "$staging" -m 2>/dev/null \
  || tar -xzf "$ws_tmp/archive.tar.gz" -C "$staging"
find "$staging" -type f | while IFS= read -r f; do
  rel="${{f#"$staging"/}}"
  rel="${{rel#./}}"
  dest="{dest_root}/$rel"
  mkdir -p "$(dirname "$dest")"
  cp -f "$f" "$dest"
done
"#
    );
    exec_sh_lc_stdin_as_user(
        runtime_bin,
        container_name,
        worker_exec_user,
        &script,
        tar_b64.trim().as_bytes(),
    )
    .await
}

/// Run shell script as worker user inside container. Author: kejiqing
pub async fn exec_sh_as_user(
    runtime_bin: &str,
    container_name: &str,
    worker_exec_user: &str,
    script: &str,
) -> Result<(), String> {
    exec_sh_lc_as_user(runtime_bin, container_name, worker_exec_user, script).await
}

async fn exec_sh_lc_as_user(
    runtime_bin: &str,
    container: &str,
    exec_user: &str,
    script: &str,
) -> Result<(), String> {
    let argv = ["exec", "--user", exec_user, container, "sh", "-lc", script];
    let out = runtime_exec(runtime_bin, &argv)
        .await
        .map_err(|e| format!("{runtime_bin} exec: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exec failed: {}",
            runtime_bin,
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

async fn exec_sh_lc_stdin_as_user(
    runtime_bin: &str,
    container: &str,
    exec_user: &str,
    script: &str,
    stdin: &[u8],
) -> Result<(), String> {
    let argv = [
        "exec", "-i", "--user", exec_user, container, "sh", "-lc", script,
    ];
    let out = runtime_exec_stdin(runtime_bin, &argv, stdin)
        .await
        .map_err(|e| format!("{runtime_bin} exec stdin: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exec stdin failed: {}",
            runtime_bin,
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

async fn exec_sh_lc_capture(
    runtime_bin: &str,
    container: &str,
    script: &str,
) -> Result<String, String> {
    let argv = ["exec", container, "sh", "-lc", script];
    let out = runtime_exec(runtime_bin, &argv)
        .await
        .map_err(|e| format!("{runtime_bin} exec: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let code = out.status.code().unwrap_or(-1);
        Err(format!(
            "{} exec capture failed (exit {code}): {}",
            runtime_bin,
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}
