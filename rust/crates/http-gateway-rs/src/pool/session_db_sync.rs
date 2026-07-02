//! e2b session readback helpers; transcript + solve timing from NAS after exec. Author: kejiqing
//! e2b solve 主路径不写 workspace gzip-tar 到 DB（legacy docker pool 除外）。

use std::path::Path;

use crate::cluster_identity;
use crate::persistence::transcript::{
    now_ms, reconcile_session_transcript_from_jsonl, JsonlMessage,
};
use crate::session_db::GatewaySessionDb;
use gateway_solve_turn::multi_agent::ORCHESTRATION_EVENTS_REL;
use gateway_solve_turn::SOLVE_TIMING_EVENTS_REL;
use serde_json::Value;
use tracing::warn;

use super::NasLayoutBackend;

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
pub fn gateway_proj_work_dir(work_root: &Path, proj_id: i64) -> Result<std::path::PathBuf, String> {
    Ok(proj_work_dir(work_root, &nas_cluster_id()?, proj_id))
}

/// Gateway convenience: session home under cluster-prefixed NAS tree.
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
    work_root.join(cluster_id).join(format!("proj_{proj_id}"))
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

fn claw_rel_file_name(rel: &str) -> &str {
    rel.trim_start_matches(".claw/").trim_start_matches('/')
}

/// NAS session artifacts → `solve_timing_jsonb` (tool/llm/orchestration/progress swimlanes).
async fn readback_turn_solve_timing_from_session_home(
    db: &GatewaySessionDb,
    nas_layout: &NasLayoutBackend,
    proj_id: i64,
    session_segment: &str,
    turn_id: &str,
) -> Result<(), String> {
    let solve_timing = nas_layout
        .read_session_claw_utf8(
            proj_id,
            session_segment,
            claw_rel_file_name(SOLVE_TIMING_EVENTS_REL),
        )
        .await?
        .unwrap_or_default();
    let orchestration = nas_layout
        .read_session_claw_utf8(
            proj_id,
            session_segment,
            claw_rel_file_name(ORCHESTRATION_EVENTS_REL),
        )
        .await?
        .unwrap_or_default();
    if !solve_timing.is_empty() || !orchestration.is_empty() {
        db.merge_turn_timing_worker_readback(turn_id, &solve_timing, &orchestration)
            .await
            .map_err(|e| format!("merge solve_timing_jsonb: {e}"))?;
    }

    let progress = nas_layout
        .read_session_claw_utf8(proj_id, session_segment, "progress-events.ndjson")
        .await?
        .unwrap_or_default();
    let task_progress = nas_layout
        .read_session_claw_utf8(proj_id, session_segment, "task-progress.json")
        .await?
        .unwrap_or_default();
    if !progress.is_empty() || !task_progress.is_empty() {
        db.replace_turn_progress_snapshot(turn_id, &progress, &task_progress)
            .await
            .map_err(|e| format!("replace turn progress snapshot: {e}"))?;
    }
    Ok(())
}

/// NAS session home → DB: transcript (`cc_messages`) + solve timing (`solve_timing_jsonb`).
/// Gateway reads through nas-api (never the gateway-local workspace disk). Author: kejiqing
pub async fn readback_turn_from_session_home(
    db: &GatewaySessionDb,
    nas_layout: &NasLayoutBackend,
    session_id: &str,
    proj_id: i64,
    turn_id: &str,
    user_prompt: &str,
) -> Result<Vec<JsonlMessage>, String> {
    let session_segment = crate::session_merge::sessions_directory_segment(session_id);
    let jsonl = nas_layout
        .read_session_jsonl(proj_id, &session_segment)
        .await?
        .unwrap_or_default();
    let messages = reconcile_session_transcript_from_jsonl(
        db,
        session_id,
        proj_id,
        &jsonl,
        turn_id,
        user_prompt,
    )
    .await
    .map_err(|e| format!("reconcile cc_messages from jsonl: {e}"))?;

    if let Err(e) = readback_turn_solve_timing_from_session_home(
        db,
        nas_layout,
        proj_id,
        &session_segment,
        turn_id,
    )
    .await
    {
        warn!(
            target: "claw_gateway_e2b_readback",
            turn_id = %turn_id,
            session_id = %session_id,
            error = %e,
            "solve timing readback from NAS failed (transcript readback succeeded)"
        );
    }

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
