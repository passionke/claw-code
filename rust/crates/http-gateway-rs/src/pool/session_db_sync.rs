//! Materialize session files from PG into worker `/claw_host_root`; read back after exec. Author: kejiqing

use std::path::Path;

use claw_sandbox_client::SandboxRpcClient;
use claw_sandbox_protocol::{
    GuestExecActor, GuestVolume, GUEST_WIPE_DS_SH, GUEST_WIPE_WORK_ROOT_SH,
};
use runtime::builtin_system_prompt_scaffold_default;

use crate::persistence::transcript::{import_turn_messages_to_db, now_ms, JsonlMessage};
use crate::pool::docker_cli::{runtime_exec, runtime_exec_stdin};
use crate::project_config_apply;
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
    pub proj_id: i64,
    pub turn_id: String,
}

/// Wipe ephemeral tmpfs before materialize: `/claw_ds` as root, `/claw_host_root` as worker user. Author: kejiqing
async fn wipe_guest_ephemeral_mounts(
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

/// Write session task/jsonl + workspace tar from PG before `gateway-solve-once`.
/// Project config (skills/rules/CLAUDE) is read from `/claw_ds` bind, not copied here. Author: kejiqing
pub async fn materialize_in(
    runtime_bin: &str,
    _work_root_host: &Path,
    container_name: &str,
    db: &GatewaySessionDb,
    input: &MaterializeInput,
    worker_exec_user: &str,
) -> Result<(), String> {
    wipe_guest_ephemeral_mounts(runtime_bin, container_name, worker_exec_user).await?;

    let workspace_tar_b64 = db
        .get_latest_workspace_tar_b64(
            &input.session_id,
            input.proj_id,
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
    exec_sh_lc_as_user(
        runtime_bin,
        container_name,
        worker_exec_user,
        project_config_apply::guest_prepare_worker_native_paths_shell(),
    )
    .await?;

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
        .render_session_jsonl(&input.session_id, input.proj_id)
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
    if GatewaySessionDb::session_jsonl_has_messages(&jsonl_body) {
        writes.push((
            format!("{GUEST_WORK_ROOT}/.claw/gateway-solve-session.jsonl",),
            jsonl_body.into_bytes(),
        ));
    }
    if let Some(row) = db
        .get_project_config(input.proj_id)
        .await
        .map_err(|e| format!("load project_config: {e}"))?
    {
        let orch = project_config_apply::solve_orchestration_marker_bytes(&row)
            .map_err(|e| format!("solve-orchestration bytes proj {}: {e}", input.proj_id))?;
        writes.push((
            format!(
                "{GUEST_WORK_ROOT}/{}",
                project_config_apply::SOLVE_ORCHESTRATION_MARKER
            ),
            orch,
        ));
        let lang = project_config_apply::language_pipeline_marker_bytes(&row)
            .map_err(|e| format!("language-pipeline bytes proj {}: {e}", input.proj_id))?;
        writes.push((
            format!(
                "{GUEST_WORK_ROOT}/{}",
                project_config_apply::LANGUAGE_PIPELINE_MARKER
            ),
            lang,
        ));
    }
    for (path, bytes) in writes {
        if bytes.len() > SESSION_MANIFEST_MAX_BYTES {
            return Err(format!(
                "guest file {path} exceeds cap {SESSION_MANIFEST_MAX_BYTES} bytes"
            ));
        }
        write_file_via_exec_user(runtime_bin, container_name, worker_exec_user, &path, &bytes)
            .await?;
    }
    Ok(())
}

/// PG → sandbox guest paths before `exec_solve` (end-state RPC). Author: kejiqing
pub async fn materialize_turn_via_sandbox(
    client: &SandboxRpcClient,
    slot_index: usize,
    db: &GatewaySessionDb,
    input: &MaterializeInput,
) -> Result<(), String> {
    client.guest_wipe(slot_index).await?;

    let workspace_tar_b64 = db
        .get_latest_workspace_tar_b64(
            &input.session_id,
            input.proj_id,
            WORKSPACE_TAR_ARTIFACT_PATH,
            WORKSPACE_TAR_ARTIFACT_KIND,
        )
        .await
        .map_err(|e| format!("load workspace tar: {e}"))?;
    if let Some(ref b64) = workspace_tar_b64 {
        if !b64.trim().is_empty() {
            client
                .guest_extract_tar_b64(slot_index, GuestVolume::SessionWorkspace, "", b64)
                .await?;
        }
    }
    client.guest_prepare_session_workspace(slot_index).await?;

    if let Some(row) = db
        .get_project_config(input.proj_id)
        .await
        .map_err(|e| format!("load project_config: {e}"))?
    {
        let scaffold = builtin_system_prompt_scaffold_default();
        let writes = project_config_apply::build_guest_materialize_writes(&row, &scaffold)
            .map_err(|e| format!("build_guest_materialize_writes proj {}: {e}", input.proj_id))?;
        for write in writes {
            let rel = write.rel_path.to_string_lossy();
            if write.bytes.len() > SESSION_MANIFEST_MAX_BYTES {
                return Err(format!(
                    "project config file {rel} exceeds cap {SESSION_MANIFEST_MAX_BYTES} bytes"
                ));
            }
            client
                .guest_write(slot_index, GuestVolume::ProjectConfig, &rel, &write.bytes)
                .await?;
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
        .render_session_jsonl(&input.session_id, input.proj_id)
        .await
        .map_err(|e| format!("render session jsonl: {e}"))?;
    if jsonl_body.len() > SESSION_MANIFEST_MAX_BYTES {
        return Err(format!(
            "session transcript exceeds cap {SESSION_MANIFEST_MAX_BYTES} bytes"
        ));
    }
    let mut writes: Vec<(&str, Vec<u8>)> = vec![("gateway-solve-task.json", task_bytes)];
    if GatewaySessionDb::session_jsonl_has_messages(&jsonl_body) {
        writes.push((".claw/gateway-solve-session.jsonl", jsonl_body.into_bytes()));
    }
    for (rel, bytes) in writes {
        if bytes.len() > SESSION_MANIFEST_MAX_BYTES {
            return Err(format!(
                "session file {rel} exceeds cap {SESSION_MANIFEST_MAX_BYTES} bytes"
            ));
        }
        client
            .guest_write(slot_index, GuestVolume::SessionWorkspace, rel, &bytes)
            .await?;
    }
    client.guest_lock_project_config(slot_index).await?;
    Ok(())
}

/// Running turn: worker `.claw` progress via sandbox `guest_read`. Author: kejiqing
pub async fn sync_progress_via_sandbox(
    client: &SandboxRpcClient,
    slot_index: usize,
    db: &GatewaySessionDb,
    turn_id: &str,
) -> Result<(), String> {
    let paths = vec![
        format!("{GUEST_WORK_ROOT}/.claw/progress-events.ndjson"),
        format!("{GUEST_WORK_ROOT}/.claw/task-progress.json"),
    ];
    let files = client.guest_read(slot_index, &paths).await?;
    let progress = guest_read_text(&files, &paths[0]);
    let task_progress = guest_read_text(&files, &paths[1]);
    if progress.is_empty() && task_progress.is_empty() {
        return Ok(());
    }
    db.replace_turn_progress_snapshot(turn_id, &progress, &task_progress)
        .await
        .map_err(|e| format!("replace turn progress: {e}"))?;
    Ok(())
}

/// Import jsonl + workspace tar + timing after sandbox exec. Author: kejiqing
pub async fn readback_turn_via_sandbox(
    client: &SandboxRpcClient,
    slot_index: usize,
    db: &GatewaySessionDb,
    pool: &PgPool,
    session_id: &str,
    proj_id: i64,
    turn_id: &str,
    user_prompt: &str,
) -> Result<Vec<JsonlMessage>, String> {
    let jsonl_path = format!("{GUEST_WORK_ROOT}/.claw/gateway-solve-session.jsonl");
    let jsonl = guest_read_single_string(client, slot_index, &jsonl_path)
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
    import_turn_messages_to_db(db, session_id, proj_id, turn_id, &messages, now)
        .await
        .map_err(|e| format!("import cc_messages: {e}"))?;

    readback_workspace_tar_via_sandbox(client, slot_index, db, session_id, proj_id, turn_id, now)
        .await?;

    readback_timing_via_sandbox(client, slot_index, db, turn_id).await?;
    let _ = pool;
    Ok(messages)
}

async fn readback_timing_via_sandbox(
    client: &SandboxRpcClient,
    slot_index: usize,
    db: &GatewaySessionDb,
    turn_id: &str,
) -> Result<(), String> {
    let timing_path = format!(
        "{GUEST_WORK_ROOT}/{}",
        gateway_solve_turn::SOLVE_TIMING_EVENTS_REL
    );
    let orchestration_path = format!(
        "{GUEST_WORK_ROOT}/{}",
        gateway_solve_turn::multi_agent::ORCHESTRATION_EVENTS_REL
    );
    let progress_path = format!("{GUEST_WORK_ROOT}/.claw/progress-events.ndjson");
    let task_progress_path = format!("{GUEST_WORK_ROOT}/.claw/task-progress.json");
    let paths = vec![
        timing_path.clone(),
        orchestration_path.clone(),
        progress_path.clone(),
        task_progress_path.clone(),
    ];
    let files = client.guest_read(slot_index, &paths).await?;
    let timing = guest_read_text(&files, &timing_path);
    let orchestration = guest_read_text(&files, &orchestration_path);
    let progress = guest_read_text(&files, &progress_path);
    let task_progress = guest_read_text(&files, &task_progress_path);
    let total = timing.len() + orchestration.len() + progress.len() + task_progress.len();
    if total > SESSION_MANIFEST_MAX_BYTES {
        return Err(format!(
            "timing artifacts exceed cap {SESSION_MANIFEST_MAX_BYTES} bytes"
        ));
    }
    db.replace_turn_progress_snapshot(turn_id, &progress, &task_progress)
        .await
        .map_err(|e| format!("replace turn progress: {e}"))?;
    db.merge_turn_timing_worker_readback(turn_id, &timing, &orchestration)
        .await
        .map_err(|e| format!("merge turn timing: {e}"))
}

async fn readback_workspace_tar_via_sandbox(
    client: &SandboxRpcClient,
    slot_index: usize,
    db: &GatewaySessionDb,
    session_id: &str,
    proj_id: i64,
    turn_id: &str,
    created_at_ms: i64,
) -> Result<(), String> {
    let meta_path = format!("{GUEST_WORK_ROOT}/.claw/__ws_tar_readback_meta.txt");
    let cap = WORKSPACE_TAR_MAX_BYTES;
    let script = format!(
        r#"set -eu
tmp=$(mktemp)
list=$(mktemp)
trap 'rm -f "$tmp" "$list"' EXIT
find {GUEST_WORK_ROOT} -type f \
  ! -path '{GUEST_WORK_ROOT}/gateway-solve-task.json' \
  ! -path '{GUEST_WORK_ROOT}/.claw/*' \
  ! -path '{GUEST_WORK_ROOT}/.cursor/rules/*' \
  ! -path '{GUEST_WORK_ROOT}/CLAUDE.md' 2>/dev/null \
  | sed "s|^{GUEST_WORK_ROOT}/||" > "$list" || true
if [ ! -s "$list" ]; then
  printf '0\n' > '{meta_path}'
  exit 0
fi
tar -czf "$tmp" -C {GUEST_WORK_ROOT} -T "$list"
sz=$(wc -c < "$tmp" | tr -d ' ')
if [ "$sz" -gt {cap} ]; then exit 2; fi
printf '%s\n' "$sz" > '{meta_path}'
base64 < "$tmp" | tr -d '\n' >> '{meta_path}'
"#
    );
    if let Err(e) = client
        .guest_exec_sh(slot_index, &script, GuestExecActor::SlotWorker)
        .await
    {
        if e.contains("exit") && e.contains('2') {
            return Err(format!(
                "workspace tar exceeds cap {WORKSPACE_TAR_MAX_BYTES} bytes"
            ));
        }
        return Err(e);
    }
    let out = guest_read_single_string(client, slot_index, &meta_path)
        .await
        .unwrap_or_default();
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
        proj_id,
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

fn guest_read_text(files: &[(String, Vec<u8>)], path: &str) -> String {
    files
        .iter()
        .find(|(p, _)| p == path)
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
        .unwrap_or_default()
}

async fn guest_read_single_string(
    client: &SandboxRpcClient,
    slot_index: usize,
    path: &str,
) -> Result<String, String> {
    let files = client.guest_read(slot_index, &[path.to_string()]).await?;
    Ok(guest_read_text(&files, path))
}

/// Import jsonl turn segment + workspace tar + timing after exec. Author: kejiqing
pub async fn readback_out(
    runtime_bin: &str,
    container_name: &str,
    db: &GatewaySessionDb,
    pool: &PgPool,
    session_id: &str,
    proj_id: i64,
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
    import_turn_messages_to_db(db, session_id, proj_id, turn_id, &messages, now)
        .await
        .map_err(|e| format!("import cc_messages: {e}"))?;

    readback_workspace_tar(
        runtime_bin,
        container_name,
        db,
        session_id,
        proj_id,
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
    let (progress, task_progress) =
        read_worker_progress_artifacts(runtime_bin, container_name).await;
    let total = timing.len() + orchestration.len() + progress.len() + task_progress.len();
    if total > SESSION_MANIFEST_MAX_BYTES {
        return Err(format!(
            "timing artifacts exceed cap {SESSION_MANIFEST_MAX_BYTES} bytes"
        ));
    }
    db.replace_turn_progress_snapshot(turn_id, &progress, &task_progress)
        .await
        .map_err(|e| format!("replace turn progress: {e}"))?;
    db.merge_turn_timing_worker_readback(turn_id, &timing, &orchestration)
        .await
        .map_err(|e| format!("merge turn timing: {e}"))
}

/// Read `.claw/progress-events.ndjson` + `task-progress.json` from a live worker. Author: kejiqing
pub async fn read_worker_progress_artifacts(
    runtime_bin: &str,
    container_name: &str,
) -> (String, String) {
    let progress = read_file_from_container(
        runtime_bin,
        container_name,
        &format!("{GUEST_WORK_ROOT}/.claw/progress-events.ndjson"),
    )
    .await
    .unwrap_or_default();
    let task_progress = read_file_from_container(
        runtime_bin,
        container_name,
        &format!("{GUEST_WORK_ROOT}/.claw/task-progress.json"),
    )
    .await
    .unwrap_or_default();
    (progress, task_progress)
}

async fn readback_workspace_tar(
    runtime_bin: &str,
    container_name: &str,
    db: &GatewaySessionDb,
    session_id: &str,
    proj_id: i64,
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
  ! -path '{GUEST_WORK_ROOT}/.claw/*' \
  ! -path '{GUEST_WORK_ROOT}/.cursor/rules/*' \
  ! -path '{GUEST_WORK_ROOT}/CLAUDE.md' 2>/dev/null \
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
        proj_id,
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
    proj_id: i64,
    session_id: &str,
) -> std::path::PathBuf {
    let seg = crate::session_merge::sessions_directory_segment(session_id);
    work_root
        .join(format!("proj_{proj_id}"))
        .join("sessions")
        .join(seg)
}
