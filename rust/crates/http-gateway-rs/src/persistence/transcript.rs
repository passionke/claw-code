//! Transcript import/export between jsonl and `cc_messages`. Author: kejiqing
#![allow(clippy::too_many_arguments)]

use std::path::Path;

use gateway_solve_turn::{
    gateway_solve_session_persistence_path, strip_report_start_marker,
    ASSISTANT_STREAM_REPORT_START_MARKER,
};
use serde_json::Value;
use sqlx::PgPool;

use crate::biz_advice_report::report_body_from_solve_output;
use crate::session_db::GatewaySessionDb;

/// One persisted message line for a turn.
#[derive(Debug, Clone)]
pub struct JsonlMessage {
    pub role: String,
    pub blocks: Value,
    pub usage: Option<Value>,
}

/// Split session jsonl into per-user-turn message groups.
#[must_use]
pub fn turn_message_groups_from_jsonl_contents(contents: &str) -> Vec<Vec<JsonlMessage>> {
    let mut groups: Vec<Vec<JsonlMessage>> = Vec::new();
    let mut current: Vec<JsonlMessage> = Vec::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(msg) = record.get("message") else {
            continue;
        };
        let role = msg
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string();
        if role == "user" && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        let blocks = msg
            .get("blocks")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let usage = msg.get("usage").cloned();
        current.push(JsonlMessage {
            role,
            blocks,
            usage,
        });
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

#[must_use]
pub fn segments_from_jsonl_path(session_home: &Path) -> Vec<Vec<JsonlMessage>> {
    let path = gateway_solve_session_persistence_path(session_home);
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    turn_message_groups_from_jsonl_contents(&contents)
}

/// Legacy segment helper for tests / report preview.
#[derive(Debug, Clone)]
pub struct JsonlTurnSegment {
    pub user_prompt: Option<String>,
    pub assistant_parts: Vec<String>,
}

#[must_use]
pub fn report_body_from_segment(segment: &JsonlTurnSegment) -> String {
    let text = segment.assistant_parts.join("\n");
    strip_report_start_marker(&text)
}

/// Formal report body from last turn messages in jsonl.
#[must_use]
pub fn report_body_from_turn_messages(messages: &[JsonlMessage]) -> String {
    let mut parts = Vec::new();
    for m in messages {
        if m.role == "assistant" {
            if let Some(arr) = m.blocks.as_array() {
                for b in arr {
                    if b.get("type").and_then(Value::as_str) == Some("text") {
                        if let Some(t) = b.get("text").and_then(Value::as_str) {
                            parts.push(t.to_string());
                        }
                    }
                }
            }
        }
    }
    strip_report_start_marker(&parts.join("\n"))
}

pub async fn import_turn_messages_to_db(
    db: &GatewaySessionDb,
    session_id: &str,
    ds_id: i64,
    turn_id: &str,
    messages: &[JsonlMessage],
    created_at_ms: i64,
) -> Result<(), sqlx::Error> {
    db.delete_messages_for_turn(turn_id).await?;
    let iteration_id = db
        .ensure_runtime_iteration(turn_id, 0, created_at_ms)
        .await?;
    for (seq, msg) in messages.iter().enumerate() {
        let seq_i32 = i32::try_from(seq).unwrap_or(i32::MAX);
        let iter = if msg.role == "user" && seq == 0 {
            None
        } else {
            Some(iteration_id)
        };
        db.insert_message(
            session_id,
            ds_id,
            turn_id,
            iter,
            seq_i32,
            &msg.role,
            &msg.blocks,
            msg.usage.as_ref(),
            created_at_ms + i64::from(seq_i32),
        )
        .await?;
    }
    Ok(())
}

async fn insert_model_usage_from_solve_json(
    db: &GatewaySessionDb,
    turn_id: &str,
    output_json: &Value,
    model: Option<&str>,
    duration_ms: i64,
) -> Result<(), sqlx::Error> {
    let model_name = output_json
        .get("model")
        .and_then(Value::as_str)
        .or(model)
        .unwrap_or("unknown");
    let usage = output_json.get("usage");
    let input_tokens = i32::try_from(
        usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
    )
    .unwrap_or(i32::MAX);
    let output_tokens = i32::try_from(
        usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
    )
    .unwrap_or(i32::MAX);
    let cache_creation = i32::try_from(
        usage
            .and_then(|u| u.get("cache_creation_input_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
    )
    .unwrap_or(i32::MAX);
    let cache_read = i32::try_from(
        usage
            .and_then(|u| u.get("cache_read_input_tokens"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
    )
    .unwrap_or(i32::MAX);
    db.insert_model_usage(
        turn_id,
        None,
        model_name,
        input_tokens,
        output_tokens,
        cache_creation,
        cache_read,
        Some(duration_ms),
        "solve",
    )
    .await
}

/// After solve: sync latest jsonl turn and persist turn result columns.
pub async fn persist_turn_after_solve(
    pool: &PgPool,
    db: &GatewaySessionDb,
    session_id: &str,
    ds_id: i64,
    turn_id: &str,
    user_prompt: &str,
    session_home: &Path,
    claw_exit_code: i32,
    output_text: &str,
    output_json: Option<&Value>,
    duration_ms: i64,
    model: Option<&str>,
) -> Result<(), sqlx::Error> {
    let workspace_rel = format!("ds_{ds_id}");
    db.upsert_project(ds_id, &format!("ds_{ds_id}"), &workspace_rel)
        .await?;

    db.update_turn_user_prompt(turn_id, user_prompt).await?;

    let groups = segments_from_jsonl_path(session_home);
    let messages = groups.last().cloned().unwrap_or_else(|| {
        vec![JsonlMessage {
            role: "user".to_string(),
            blocks: serde_json::json!([{"type":"text","text":user_prompt}]),
            usage: None,
        }]
    });
    let now = now_ms();
    import_turn_messages_to_db(db, session_id, ds_id, turn_id, &messages, now).await?;

    let report_message = if let Some(json) = output_json {
        report_body_from_solve_output(output_text, Some(json)).ok()
    } else {
        report_body_from_solve_output(output_text, None).ok()
    };
    let body_from_jsonl = report_body_from_turn_messages(&messages);
    let has_report = report_message
        .as_ref()
        .is_some_and(|m| !m.trim().is_empty())
        || !body_from_jsonl.trim().is_empty()
        || messages.iter().any(|m| {
            m.blocks
                .to_string()
                .contains(ASSISTANT_STREAM_REPORT_START_MARKER)
        });

    db.finish_turn(
        turn_id,
        claw_exit_code,
        report_message.as_deref(),
        output_json,
        has_report,
    )
    .await?;

    if let Some(json) = output_json {
        insert_model_usage_from_solve_json(db, turn_id, json, model, duration_ms).await?;
    }

    let mount = session_home.display().to_string();
    let finished = now_ms();
    let started = finished.saturating_sub(duration_ms);
    db.upsert_turn_container_run(
        turn_id,
        &mount,
        started,
        finished,
        duration_ms,
        None,
        None,
        None,
    )
    .await?;

    let _ = pool;
    Ok(())
}

/// Load session messages from DB and write gateway jsonl for worker续聊.
pub async fn ensure_jsonl_from_db(
    db: &GatewaySessionDb,
    session_id: &str,
    ds_id: i64,
    session_home: &Path,
) -> Result<(), sqlx::Error> {
    let rows = db.list_messages_for_session(session_id, ds_id).await?;
    if rows.is_empty() {
        return Ok(());
    }
    let path = gateway_solve_session_persistence_path(session_home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            sqlx::Error::Io(std::io::Error::other(format!("create .claw dir: {e}")))
        })?;
    }
    let mut lines = Vec::new();
    let now = now_ms();
    lines.push(
        serde_json::json!({
            "type": "session_meta",
            "session_id": format!("session-{now}"),
            "version": 1,
            "created_at_ms": now,
            "updated_at_ms": now,
        })
        .to_string(),
    );
    for row in rows {
        let mut message = serde_json::json!({
            "role": row.role,
            "blocks": row.blocks,
        });
        if let Some(usage) = row.usage {
            message["usage"] = usage;
        }
        lines.push(
            serde_json::json!({
                "type": "message",
                "message": message,
            })
            .to_string(),
        );
    }
    let body = lines.join("\n");
    if !body.is_empty() {
        let body = format!("{body}\n");
        std::fs::write(&path, body)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(format!("write jsonl: {e}"))))?;
    }
    Ok(())
}

#[must_use]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
