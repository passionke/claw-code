//! Materialize session files from PG into NAS/host paths; read back after FC exec. Author: kejiqing

use std::path::Path;

use crate::persistence::transcript::{import_turn_messages_to_db, now_ms, JsonlMessage};
use crate::project_config_apply;
use crate::session_db::GatewaySessionDb;
use serde_json::Value;
use sqlx::PgPool;

pub const GUEST_WORK_ROOT: &str = "/claw_host_root";
pub const DS_MOUNT_TARGET: &str = "/claw_ds";
#[allow(dead_code)]
pub const WORKSPACE_TAR_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const SESSION_MANIFEST_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const WORKSPACE_TAR_ARTIFACT_PATH: &str = "__workspace_tar_gz__";
pub const WORKSPACE_TAR_ARTIFACT_KIND: &str = "workspace_tar_gz";

#[derive(Debug, Clone)]
pub struct MaterializeInput {
    pub session_id: String,
    pub proj_id: i64,
    pub turn_id: String,
}

/// Host path `work_root/proj_<id>/` (gateway view; same tree as `git pull`). Author: kejiqing
#[must_use]
pub fn proj_work_dir(work_root: &Path, proj_id: i64) -> std::path::PathBuf {
    work_root.join(format!("proj_{proj_id}"))
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

/// PG project_config markers written under `/claw_host_root` each solve. Author: kejiqing
pub fn guest_session_marker_writes(
    row: &crate::session_db::ProjectConfigRow,
) -> Result<Vec<(String, Vec<u8>)>, String> {
    let orch = project_config_apply::solve_orchestration_marker_bytes(row)
        .map_err(|e| format!("solve-orchestration: {e}"))?;
    let lang = project_config_apply::language_pipeline_marker_bytes(row)
        .map_err(|e| format!("language-pipeline: {e}"))?;
    let preflight = project_config_apply::solve_preflight_marker_bytes(row)
        .map_err(|e| format!("solve-preflight: {e}"))?;
    Ok(vec![
        (
            format!(
                "{GUEST_WORK_ROOT}/{}",
                project_config_apply::SOLVE_ORCHESTRATION_MARKER
            ),
            orch,
        ),
        (
            format!(
                "{GUEST_WORK_ROOT}/{}",
                project_config_apply::LANGUAGE_PIPELINE_MARKER
            ),
            lang,
        ),
        (
            format!(
                "{GUEST_WORK_ROOT}/{}",
                project_config_apply::SOLVE_PREFLIGHT_MARKER
            ),
            preflight,
        ),
    ])
}

/// PG → host session paths for FC NAS-backed sandboxes (shared with gateway). Author: kejiqing
pub async fn materialize_turn_via_sandbox_host_paths(
    db: &GatewaySessionDb,
    work_root: &Path,
    _proj_work_dir: &Path,
    input: &MaterializeInput,
) -> Result<(), String> {
    use tokio::fs;

    let session_home = session_home_under_work_root(work_root, input.proj_id, &input.session_id);
    fs::create_dir_all(session_home.join(".claw"))
        .await
        .map_err(|e| format!("mkdir session .claw: {e}"))?;

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
    fs::write(session_home.join("gateway-solve-task.json"), &task_bytes)
        .await
        .map_err(|e| format!("write gateway-solve-task.json: {e}"))?;

    let jsonl_body = db
        .render_session_jsonl(&input.session_id, input.proj_id)
        .await
        .map_err(|e| format!("render session jsonl: {e}"))?;
    if jsonl_body.len() > SESSION_MANIFEST_MAX_BYTES {
        return Err(format!(
            "session transcript exceeds cap {SESSION_MANIFEST_MAX_BYTES} bytes"
        ));
    }
    if GatewaySessionDb::session_jsonl_has_messages(&jsonl_body) {
        fs::write(
            session_home.join(".claw/gateway-solve-session.jsonl"),
            jsonl_body.as_bytes(),
        )
        .await
        .map_err(|e| format!("write session jsonl: {e}"))?;
    }

    if let Ok(Some(row)) = db.get_project_config(input.proj_id).await {
        for (path, bytes) in guest_session_marker_writes(&row)
            .map_err(|e| format!("guest session markers proj {} (host): {e}", input.proj_id))?
        {
            let rel = path
                .strip_prefix(&format!("{GUEST_WORK_ROOT}/"))
                .unwrap_or(path.as_str());
            let dest = session_home.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("mkdir marker parent: {e}"))?;
            }
            fs::write(&dest, bytes)
                .await
                .map_err(|e| format!("write marker {rel}: {e}"))?;
        }
    }
    Ok(())
}

/// Build exec script to push solve session files into FC sandbox (self-hosted dev without shared NAS).
#[allow(dead_code)]
pub async fn build_fc_solve_materialize_script(
    db: &GatewaySessionDb,
    work_root: &Path,
    input: &MaterializeInput,
) -> Result<String, String> {
    use super::interactive_backend::build_fc_guest_writes_script;

    let session_home = session_home_under_work_root(work_root, input.proj_id, &input.session_id);
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
    let mut files: Vec<(String, Vec<u8>)> =
        vec![("gateway-solve-task.json".to_string(), task_bytes)];
    let settings_path = session_home.join(".claw/settings.json");
    if settings_path.is_file() {
        let settings = tokio::fs::read(&settings_path)
            .await
            .map_err(|e| format!("read session settings.json: {e}"))?;
        if settings.len() <= SESSION_MANIFEST_MAX_BYTES {
            files.push((".claw/settings.json".to_string(), settings));
        }
    }
    let jsonl_body = db
        .render_session_jsonl(&input.session_id, input.proj_id)
        .await
        .map_err(|e| format!("render session jsonl: {e}"))?;
    if jsonl_body.len() <= SESSION_MANIFEST_MAX_BYTES
        && GatewaySessionDb::session_jsonl_has_messages(&jsonl_body)
    {
        files.push((
            ".claw/gateway-solve-session.jsonl".to_string(),
            jsonl_body.into_bytes(),
        ));
    }
    if let Ok(Some(row)) = db.get_project_config(input.proj_id).await {
        for (path, bytes) in guest_session_marker_writes(&row).map_err(|e| {
            format!(
                "guest session markers proj {} (fc exec): {e}",
                input.proj_id
            )
        })? {
            let rel = path
                .strip_prefix(&format!("{GUEST_WORK_ROOT}/"))
                .map(std::string::ToString::to_string)
                .unwrap_or(path);
            if bytes.len() <= SESSION_MANIFEST_MAX_BYTES {
                files.push((rel, bytes));
            }
        }
    }
    Ok(build_fc_guest_writes_script(GUEST_WORK_ROOT, &files))
}

pub async fn readback_turn_from_session_home(
    db: &GatewaySessionDb,
    pool: &PgPool,
    work_root: &Path,
    session_id: &str,
    proj_id: i64,
    turn_id: &str,
    user_prompt: &str,
) -> Result<Vec<JsonlMessage>, String> {
    use tokio::fs;

    let session_home = session_home_under_work_root(work_root, proj_id, session_id);
    let jsonl_path = session_home.join(".claw/gateway-solve-session.jsonl");
    let jsonl = fs::read_to_string(&jsonl_path).await.unwrap_or_default();
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
    let _ = pool;
    Ok(messages)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_db::ProjectConfigRow;
    use serde_json::{json, Value};

    fn test_row(solve_preflight_json: Value) -> ProjectConfigRow {
        ProjectConfigRow {
            proj_id: 27,
            content_rev: "rev-sync".into(),
            stable_content_rev: Some("rev-sync".into()),
            draft_open: false,
            updated_at_ms: 0,
            rules_json: json!([]),
            mcp_servers_json: json!({}),
            skills_sources_json: json!([]),
            skills_json: json!([]),
            allowed_tools_json: json!([]),
            claude_md: None,
            git_sync_json: json!({}),
            solve_preflight_json,
            solve_orchestration_json: json!({"kind": "single_turn"}),
            language_pipeline_json: json!({}),
            extra_session_fields_json: json!([]),
            prompt_limits_json: json!({}),
            worker_isolation_json: json!({"mode": "strict"}),
        }
    }

    #[test]
    fn guest_session_marker_writes_include_preflight_tombstone_when_pg_none() {
        let writes =
            guest_session_marker_writes(&test_row(json!({"kind": "none"}))).expect("marker writes");
        let preflight = writes
            .iter()
            .find(|(path, _)| path.ends_with("solve-preflight.json"))
            .expect("preflight write");
        assert_eq!(
            preflight.0,
            "/claw_host_root/home/.claw/solve-preflight.json"
        );
        let parsed: Value = serde_json::from_slice(&preflight.1).expect("json");
        assert_eq!(parsed.get("kinds").and_then(Value::as_array), Some(&vec![]));
    }

    #[test]
    fn guest_session_marker_writes_include_sqlbot_preflight_when_enabled() {
        let writes = guest_session_marker_writes(&test_row(json!({"kind": "sqlbot_mcp_start"})))
            .expect("marker writes");
        let preflight = writes
            .iter()
            .find(|(path, _)| path.ends_with("solve-preflight.json"))
            .expect("preflight write");
        let parsed: Value = serde_json::from_slice(&preflight.1).expect("json");
        assert_eq!(
            parsed.get("kinds").and_then(Value::as_array),
            Some(&vec![json!("sqlbot_mcp_start")])
        );
    }
}
