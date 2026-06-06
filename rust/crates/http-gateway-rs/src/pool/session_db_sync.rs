//! Materialize session files from PG into worker `/claw_host_root`; read back after exec. Author: kejiqing

use std::path::Path;

use crate::gateway_global_settings;
use crate::persistence::transcript::{import_turn_messages_to_db, now_ms, JsonlMessage};
use crate::pool::docker_cli::{runtime_exec, runtime_exec_stdin};
use crate::project_config_apply::{self, GuestMaterializeWrite};
use crate::project_config_draft;
use crate::session_db::GatewaySessionDb;
use serde_json::Value;
use sqlx::PgPool;

pub const GUEST_WORK_ROOT: &str = "/claw_host_root";
pub const DS_MOUNT_TARGET: &str = "/claw_ds";
/// Per-turn workspace tar.gz cap (pool v1). Author: kejiqing
pub const WORKSPACE_TAR_MAX_BYTES: usize = 16 * 1024 * 1024;
/// Task/jsonl/settings cap (separate from workspace tar). Author: kejiqing
pub const SESSION_MANIFEST_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const WORKSPACE_TAR_ARTIFACT_PATH: &str = "__workspace_tar_gz__";
pub const WORKSPACE_TAR_ARTIFACT_KIND: &str = "workspace_tar_gz";

#[derive(Debug, Clone)]
pub struct MaterializeInput {
    pub session_id: String,
    pub ds_id: i64,
    pub turn_id: String,
}

/// Wipe ephemeral worker workspace before PG materialize (no tmpfs residue). Author: kejiqing
async fn wipe_guest_work_root(runtime_bin: &str, container_name: &str) -> Result<(), String> {
    let script = format!("find {GUEST_WORK_ROOT} -mindepth 1 -delete 2>/dev/null || true");
    exec_sh_lc(runtime_bin, container_name, &script).await
}

/// Write task/settings/jsonl + workspace tar from PG before `gateway-solve-once`. Author: kejiqing
pub async fn materialize_in(
    runtime_bin: &str,
    _work_root_host: &Path,
    container_name: &str,
    db: &GatewaySessionDb,
    input: &MaterializeInput,
    worker_exec_user: &str,
) -> Result<(), String> {
    wipe_guest_work_root(runtime_bin, container_name).await?;

    let workspace_tar_b64 = db
        .get_latest_workspace_tar_b64(
            &input.session_id,
            input.ds_id,
            WORKSPACE_TAR_ARTIFACT_PATH,
            WORKSPACE_TAR_ARTIFACT_KIND,
        )
        .await
        .map_err(|e| format!("load workspace tar: {e}"))?;
    if let Some(ref b64) = workspace_tar_b64 {
        if !b64.trim().is_empty() {
            extract_workspace_tar_b64(runtime_bin, container_name, worker_exec_user, b64).await?;
        }
    }

    let task = db
        .get_solve_task_json(&input.turn_id)
        .await
        .map_err(|e| format!("load solve_task_json: {e}"))?
        .ok_or_else(|| format!("missing solve_task_json for turn {}", input.turn_id))?;
    let task_bytes = serde_json::to_vec(&task).map_err(|e| format!("serialize task: {e}"))?;
    if task_bytes.len() > SESSION_MANIFEST_MAX_BYTES {
        return Err(format!(
            "solve_task_json exceeds cap {SESSION_MANIFEST_MAX_BYTES} bytes"
        ));
    }
    let jsonl_body = db
        .render_session_jsonl(&input.session_id, input.ds_id)
        .await
        .map_err(|e| format!("render session jsonl: {e}"))?;
    if jsonl_body.len() > SESSION_MANIFEST_MAX_BYTES {
        return Err(format!(
            "session transcript exceeds cap {SESSION_MANIFEST_MAX_BYTES} bytes"
        ));
    }
    let mut writes: Vec<(String, Vec<u8>)> = vec![(
        format!("{GUEST_WORK_ROOT}/gateway-solve-task.json"),
        task_bytes,
    )];
    if !jsonl_body.is_empty() {
        writes.push((
            format!("{GUEST_WORK_ROOT}/.claw/gateway-solve-session.jsonl",),
            jsonl_body.into_bytes(),
        ));
    }
    append_project_config_guest_writes(db, input.ds_id, &mut writes).await?;
    for (path, bytes) in writes {
        if bytes.len() > SESSION_MANIFEST_MAX_BYTES {
            return Err(format!(
                "guest file {path} exceeds cap {SESSION_MANIFEST_MAX_BYTES} bytes"
            ));
        }
        write_file_via_exec_user(runtime_bin, container_name, worker_exec_user, &path, &bytes)
            .await?;
    }
    exec_sh_lc_as_user(
        runtime_bin,
        container_name,
        worker_exec_user,
        project_config_apply::guest_claw_compat_symlink_shell(),
    )
    .await?;
    Ok(())
}

/// Admin `project_config` (effective formal rev) → `/claw_host_root` every solve. Author: kejiqing
async fn append_project_config_guest_writes(
    db: &GatewaySessionDb,
    ds_id: i64,
    writes: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), String> {
    let Some(row) = project_config_draft::row_for_materialize(db, ds_id)
        .await
        .map_err(|e| format!("load project_config for materialize: {e}"))?
    else {
        return Ok(());
    };
    let scaffold = gateway_global_settings::load_system_prompt_default(db)
        .await
        .map_err(|e| format!("load system_prompt_default: {e}"))?;
    let guest_writes = project_config_apply::build_guest_materialize_writes(&row, &scaffold)
        .map_err(|e| format!("build guest project_config writes: {e}"))?;
    for GuestMaterializeWrite { rel_path, bytes } in guest_writes {
        let dest = format!(
            "{GUEST_WORK_ROOT}/{}",
            rel_path.to_string_lossy().replace('\\', "/")
        );
        writes.push((dest, bytes));
    }
    Ok(())
}

/// Import jsonl turn segment + workspace tar + timing after exec. Author: kejiqing
pub async fn readback_out(
    runtime_bin: &str,
    container_name: &str,
    db: &GatewaySessionDb,
    pool: &PgPool,
    session_id: &str,
    ds_id: i64,
    turn_id: &str,
    user_prompt: &str,
) -> Result<Vec<JsonlMessage>, String> {
    let jsonl = read_file_from_container(
        runtime_bin,
        container_name,
        &format!("{GUEST_WORK_ROOT}/.claw/gateway-solve-session.jsonl"),
    )
    .await
    .unwrap_or_default();
    let groups = crate::persistence::transcript::turn_message_groups_from_jsonl_contents(&jsonl);
    let messages = groups.last().cloned().unwrap_or_else(|| {
        vec![JsonlMessage {
            role: "user".to_string(),
            blocks: serde_json::json!([{"type":"text","text":user_prompt}]),
            usage: None,
        }]
    });
    let now = now_ms();
    import_turn_messages_to_db(db, session_id, ds_id, turn_id, &messages, now)
        .await
        .map_err(|e| format!("import cc_messages: {e}"))?;

    readback_workspace_tar(
        runtime_bin,
        container_name,
        db,
        session_id,
        ds_id,
        turn_id,
        now,
    )
    .await?;

    readback_timing_to_db(runtime_bin, container_name, db, turn_id).await?;
    let _ = pool;
    Ok(messages)
}

async fn readback_timing_to_db(
    runtime_bin: &str,
    container_name: &str,
    db: &GatewaySessionDb,
    turn_id: &str,
) -> Result<(), String> {
    let timing = read_file_from_container(
        runtime_bin,
        container_name,
        &format!(
            "{GUEST_WORK_ROOT}/{}",
            gateway_solve_turn::SOLVE_TIMING_EVENTS_REL
        ),
    )
    .await
    .unwrap_or_default();
    let orchestration = read_file_from_container(
        runtime_bin,
        container_name,
        &format!(
            "{GUEST_WORK_ROOT}/{}",
            gateway_solve_turn::multi_agent::ORCHESTRATION_EVENTS_REL
        ),
    )
    .await
    .unwrap_or_default();
    let progress = read_file_from_container(
        runtime_bin,
        container_name,
        &format!("{GUEST_WORK_ROOT}/.claw/progress-events.ndjson"),
    )
    .await
    .unwrap_or_default();
    let total = timing.len() + orchestration.len() + progress.len();
    if total > SESSION_MANIFEST_MAX_BYTES {
        return Err(format!(
            "timing artifacts exceed cap {SESSION_MANIFEST_MAX_BYTES} bytes"
        ));
    }
    db.merge_turn_timing_worker_readback(turn_id, &timing, &orchestration, &progress)
        .await
        .map_err(|e| format!("merge turn timing: {e}"))
}

async fn readback_workspace_tar(
    runtime_bin: &str,
    container_name: &str,
    db: &GatewaySessionDb,
    session_id: &str,
    ds_id: i64,
    turn_id: &str,
    created_at_ms: i64,
) -> Result<(), String> {
    let cap = WORKSPACE_TAR_MAX_BYTES;
    let script = format!(
        r#"set -eu
tmp=$(mktemp)
list=$(mktemp)
trap 'rm -f "$tmp" "$list"' EXIT
find {GUEST_WORK_ROOT} -type f \
  ! -path '{GUEST_WORK_ROOT}/gateway-solve-task.json' \
  ! -path '{GUEST_WORK_ROOT}/.claw/*' 2>/dev/null \
  | sed "s|^{GUEST_WORK_ROOT}/||" > "$list" || true
if [ ! -s "$list" ]; then
  printf '0\n'
  exit 0
fi
tar -czf "$tmp" -C {GUEST_WORK_ROOT} -T "$list"
sz=$(wc -c < "$tmp" | tr -d ' ')
if [ "$sz" -gt {cap} ]; then exit 2; fi
printf '%s\n' "$sz"
base64 < "$tmp" | tr -d '\n'
"#
    );
    let out = match exec_sh_lc_capture(runtime_bin, container_name, &script).await {
        Ok(v) => v,
        Err(e) if e.contains("exit") && e.contains('2') => {
            return Err(format!(
                "workspace tar exceeds cap {WORKSPACE_TAR_MAX_BYTES} bytes"
            ));
        }
        Err(e) => return Err(e),
    };
    let mut lines = out.splitn(2, '\n');
    let size_line = lines.next().unwrap_or("0").trim();
    let raw_size: usize = size_line
        .parse()
        .map_err(|_| format!("invalid workspace tar size line: {size_line}"))?;
    if raw_size == 0 {
        return Ok(());
    }
    let tar_b64 = lines.next().unwrap_or("").trim();
    if tar_b64.is_empty() {
        return Err("workspace tar base64 empty".into());
    }
    db.upsert_workspace_tar_b64(
        session_id,
        ds_id,
        turn_id,
        WORKSPACE_TAR_ARTIFACT_PATH,
        WORKSPACE_TAR_ARTIFACT_KIND,
        tar_b64,
        raw_size,
        created_at_ms,
    )
    .await
    .map_err(|e| format!("upsert workspace tar: {e}"))?;
    Ok(())
}

async fn extract_workspace_tar_b64(
    runtime_bin: &str,
    container_name: &str,
    worker_exec_user: &str,
    tar_b64: &str,
) -> Result<(), String> {
    // Extract to tmp then cp files — avoids tar utime/chmod on {GUEST_WORK_ROOT} (macOS podman). Author: kejiqing
    let script = format!(
        r#"set -eu
ws_tmp=$(mktemp -d)
staging="$ws_tmp/staging"
mkdir -p "$staging"
trap 'rm -rf "$ws_tmp"' EXIT
base64 -d > "$ws_tmp/archive.tar.gz"
tar -xzf "$ws_tmp/archive.tar.gz" -C "$staging" -m --no-same-owner --no-same-permissions 2>/dev/null \
  || tar -xzf "$ws_tmp/archive.tar.gz" -C "$staging" -m 2>/dev/null \
  || tar -xzf "$ws_tmp/archive.tar.gz" -C "$staging"
find "$staging" -type f | while IFS= read -r f; do
  rel="${{f#"$staging"/}}"
  rel="${{rel#./}}"
  dest="{GUEST_WORK_ROOT}/$rel"
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

/// Mark turn ready for next enqueue; sets terminal `succeeded` in same transaction. Author: kejiqing
pub async fn finalize_turn_after_readback(
    db: &GatewaySessionDb,
    turn_id: &str,
    claw_exit_code: i32,
    report_message: Option<&str>,
    output_json: Option<&Value>,
) -> Result<(), String> {
    db.finalize_turn_with_artifacts_ready(
        turn_id,
        if claw_exit_code == 0 {
            "succeeded"
        } else {
            "failed"
        },
        Some(now_ms()),
        claw_exit_code,
        report_message,
        output_json,
        claw_exit_code == 0,
    )
    .await
    .map_err(|e| format!("finalize turn: {e}"))
}

async fn write_file_via_exec_user(
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

async fn read_file_from_container(
    runtime_bin: &str,
    container: &str,
    path: &str,
) -> Result<String, String> {
    let script = format!("cat '{path}' 2>/dev/null || true");
    let out = exec_sh_lc_capture(runtime_bin, container, &script).await?;
    Ok(out)
}

async fn exec_sh_lc(runtime_bin: &str, container: &str, script: &str) -> Result<(), String> {
    exec_sh_lc_as_user(runtime_bin, container, "root", script).await
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

#[must_use]
pub fn session_home_under_work_root(
    work_root: &Path,
    ds_id: i64,
    session_id: &str,
) -> std::path::PathBuf {
    let seg = crate::session_merge::sessions_directory_segment(session_id);
    work_root
        .join(format!("ds_{ds_id}"))
        .join("sessions")
        .join(seg)
}
