//! FC/E2B session transcript helpers; read back from NAS session roots after exec. Author: kejiqing

use std::path::Path;

use crate::cluster_identity;
use crate::persistence::transcript::{import_turn_messages_to_db, now_ms, JsonlMessage};
use crate::session_db::GatewaySessionDb;
use serde_json::Value;
use sqlx::PgPool;

pub const GUEST_CLAW_SESSIONS: &str = "/claw_sessions";
pub const DS_MOUNT_TARGET: &str = "/claw_ds";
#[allow(dead_code)]
pub const WORKSPACE_TAR_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const SESSION_MANIFEST_MAX_BYTES: usize = 16 * 1024 * 1024;
pub const WORKSPACE_TAR_ARTIFACT_PATH: &str = "__workspace_tar_gz__";
pub const WORKSPACE_TAR_ARTIFACT_KIND: &str = "workspace_tar_gz";

/// Resolve `CLAW_CLUSTER_ID` for NAS path operations. Author: kejiqing
pub fn nas_cluster_id() -> Result<String, String> {
    cluster_identity::gateway_cluster_id()
}

/// Gateway convenience: `work_root/{clusterId}/proj_{id}/`.
#[must_use]
pub fn gateway_proj_work_dir(work_root: &Path, proj_id: i64) -> Result<std::path::PathBuf, String> {
    Ok(proj_work_dir(work_root, &nas_cluster_id()?, proj_id))
}

/// Gateway convenience: session home under cluster-prefixed NAS tree.
#[must_use]
pub fn gateway_session_home(
    work_root: &Path,
    proj_id: i64,
    session_id: &str,
) -> Result<std::path::PathBuf, String> {
    Ok(session_home_under_work_root(
        work_root,
        &nas_cluster_id()?,
        proj_id,
        session_id,
    ))
}

/// Host path `work_root/{clusterId}/proj_<id>/` (gateway view). Author: kejiqing
#[must_use]
pub fn proj_work_dir(work_root: &Path, cluster_id: &str, proj_id: i64) -> std::path::PathBuf {
    work_root
        .join(cluster_id)
        .join(format!("proj_{proj_id}"))
}

#[must_use]
pub fn session_home_under_work_root(
    work_root: &Path,
    cluster_id: &str,
    proj_id: i64,
    session_id: &str,
) -> std::path::PathBuf {
    let seg = crate::session_merge::sessions_directory_segment(session_id);
    work_root
        .join(cluster_id)
        .join(format!("proj_{proj_id}"))
        .join("sessions")
        .join(seg)
}

/// Empty `session_meta` jsonl for first-turn solve (overwrites stale worker-root files). Author: kejiqing
#[must_use]
pub fn bootstrap_empty_solve_session_jsonl(session_id: &str, session_segment: &str) -> String {
    let workspace = format!("{GUEST_CLAW_SESSIONS}/{session_segment}");
    let line = serde_json::json!({
        "type": "session_meta",
        "session_id": session_id,
        "version": 1,
        "created_at_ms": 0_i64,
        "updated_at_ms": 0_i64,
        "workspace_root": workspace,
    });
    format!("{line}\n")
}

pub async fn readback_turn_from_session_home(
    db: &GatewaySessionDb,
    pool: &PgPool,
    work_root: &Path,
    cluster_id: &str,
    session_id: &str,
    proj_id: i64,
    turn_id: &str,
    user_prompt: &str,
) -> Result<Vec<JsonlMessage>, String> {
    use tokio::fs;

    let session_home = session_home_under_work_root(work_root, cluster_id, proj_id, session_id);
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

