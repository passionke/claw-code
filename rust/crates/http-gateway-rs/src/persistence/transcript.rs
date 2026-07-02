//! Transcript import/export between jsonl and `cc_messages`. Author: kejiqing
#![allow(clippy::too_many_arguments)]

use std::path::Path;

use gateway_solve_turn::gateway_solve_session_persistence_path;
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

/// Parse all `type:message` lines from session jsonl (order preserved).
#[must_use]
pub fn messages_from_jsonl_contents(contents: &str) -> Vec<JsonlMessage> {
    let mut messages = Vec::new();
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
        let blocks = msg
            .get("blocks")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let usage = msg.get("usage").cloned();
        messages.push(JsonlMessage {
            role,
            blocks,
            usage,
        });
    }
    messages
}

/// Split session jsonl into per-user-turn message groups.
#[must_use]
pub fn turn_message_groups_from_jsonl_contents(contents: &str) -> Vec<Vec<JsonlMessage>> {
    let messages = messages_from_jsonl_contents(contents);
    let mut groups: Vec<Vec<JsonlMessage>> = Vec::new();
    let mut current: Vec<JsonlMessage> = Vec::new();

    for msg in messages {
        if msg.role == "user" && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        current.push(msg);
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
    segment.assistant_parts.join("\n")
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
    parts.join("\n")
}

pub async fn import_turn_messages_to_db(
    db: &GatewaySessionDb,
    session_id: &str,
    proj_id: i64,
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
            proj_id,
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

/// NAS jsonl → DB: reconcile full session transcript into `cc_messages` (one-way index). Author: kejiqing
pub async fn reconcile_session_transcript_from_jsonl(
    db: &GatewaySessionDb,
    session_id: &str,
    proj_id: i64,
    jsonl: &str,
    fallback_turn_id: &str,
    fallback_user_prompt: &str,
) -> Result<Vec<JsonlMessage>, sqlx::Error> {
    let groups = turn_message_groups_from_jsonl_contents(jsonl);
    if groups.is_empty() {
        let fallback = vec![JsonlMessage {
            role: "user".to_string(),
            blocks: serde_json::json!([{"type":"text","text":fallback_user_prompt}]),
            usage: None,
        }];
        import_turn_messages_to_db(
            db,
            session_id,
            proj_id,
            fallback_turn_id,
            &fallback,
            now_ms(),
        )
        .await?;
        return Ok(fallback);
    }

    let turns = db.list_turns_for_session(session_id, proj_id).await?;
    db.delete_messages_for_session(session_id, proj_id).await?;

    let mut imported = Vec::new();
    for (idx, group) in groups.iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        let turn_id = turns
            .get(idx)
            .map(|t| t.turn_id.as_str())
            .or_else(|| turns.last().map(|t| t.turn_id.as_str()))
            .unwrap_or(fallback_turn_id);
        let created_at_ms = turns
            .get(idx)
            .map(|t| t.created_at_ms)
            .unwrap_or_else(now_ms);
        import_turn_messages_to_db(db, session_id, proj_id, turn_id, group, created_at_ms).await?;
        if idx == groups.len() - 1 {
            imported.clone_from(group);
        }
    }
    Ok(imported)
}

fn insert_model_usage_from_solve_json(
    _db: &GatewaySessionDb,
    turn_id: &str,
    output_json: &Value,
    model: Option<&str>,
    duration_ms: i64,
) {
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
    let _ = (
        turn_id,
        model_name,
        input_tokens,
        output_tokens,
        cache_creation,
        cache_read,
        duration_ms,
    );
}

/// After solve: sync latest jsonl turn and persist turn result columns.
pub async fn persist_turn_after_solve(
    pool: &PgPool,
    db: &GatewaySessionDb,
    session_id: &str,
    proj_id: i64,
    turn_id: &str,
    user_prompt: &str,
    session_home: &Path,
    claw_exit_code: i32,
    output_text: &str,
    output_json: Option<&Value>,
    duration_ms: i64,
    model: Option<&str>,
) -> Result<(), sqlx::Error> {
    let workspace_rel = format!("proj_{proj_id}");
    db.upsert_project(proj_id, &format!("proj_{proj_id}"), &workspace_rel)
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
    import_turn_messages_to_db(db, session_id, proj_id, turn_id, &messages, now).await?;

    let report_message = if let Some(json) = output_json {
        report_body_from_solve_output(output_text, Some(json)).ok()
    } else {
        report_body_from_solve_output(output_text, None).ok()
    };
    let body_from_jsonl = report_body_from_turn_messages(&messages);
    let has_report = report_message
        .as_ref()
        .is_some_and(|m| !m.trim().is_empty())
        || !body_from_jsonl.trim().is_empty();

    db.finish_turn(
        turn_id,
        claw_exit_code,
        report_message.as_deref(),
        output_json,
        has_report,
    )
    .await?;

    if let Some(json) = output_json {
        insert_model_usage_from_solve_json(db, turn_id, json, model, duration_ms);
    }

    let _ = pool;
    let _ = session_home;
    Ok(())
}

/// Load session messages from DB and write gateway jsonl for worker续聊.
pub async fn ensure_jsonl_from_db(
    db: &GatewaySessionDb,
    session_id: &str,
    proj_id: i64,
    session_home: &Path,
) -> Result<(), sqlx::Error> {
    let body = db.render_session_jsonl(session_id, proj_id).await?;
    if !GatewaySessionDb::session_jsonl_has_messages(&body) {
        return Ok(());
    }
    let path = gateway_solve_session_persistence_path(session_home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            sqlx::Error::Io(std::io::Error::other(format!("create .claw dir: {e}")))
        })?;
    }
    std::fs::write(&path, &body)
        .map_err(|e| sqlx::Error::Io(std::io::Error::other(format!("write jsonl: {e}"))))?;
    Ok(())
}

#[must_use]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_i64, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
