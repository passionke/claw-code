//! Parse tool_use / tool_result pairs from `gateway-solve-session.jsonl` for a user turn.
//! Author: kejiqing

use crate::gateway_solve_session_persistence_path;
use crate::task_progress::{progress_events_path, ProgressEvent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;

/// One tool invocation for a user turn (jsonl order + optional progress timestamps).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnToolRecord {
    pub tool_use_id: String,
    #[serde(rename = "toolName")]
    pub name: String,
    pub input: Value,
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// 1-based order in this turn (jsonl encounter order).
    pub sequence: u32,
    /// From `progress-events.ndjson` `mcp_tool_started` when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    /// End of tool window: next tool start or turn `finished_at_ms`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
}

/// Pending tool_use not yet paired with tool_result (insertion order preserved).
struct PendingTool {
    tool_use_id: String,
    name: String,
    input: Value,
}

/// List tool executions for the given 1-based user turn index.
pub fn list_tool_executions_for_user_turn(
    session_home: &Path,
    user_turn_index: usize,
) -> Result<Vec<TurnToolRecord>, String> {
    list_tool_executions_for_user_turn_with_time_window(session_home, user_turn_index, None, None)
}

/// Parse tool_use / tool_result pairs from jsonl text (PG `render_session_jsonl` or on-disk file).
pub fn list_tool_executions_for_user_turn_from_jsonl_contents(
    jsonl_contents: &str,
    user_turn_index: usize,
) -> Result<Vec<TurnToolRecord>, String> {
    if user_turn_index == 0 {
        return Err("user_turn_index must be >= 1".to_string());
    }
    if jsonl_contents.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut prompt_seen = 0usize;
    let mut in_turn = false;
    let mut pending: Vec<PendingTool> = Vec::new();
    let mut out: Vec<TurnToolRecord> = Vec::new();
    let mut sequence: u32 = 0;

    for line in jsonl_contents.lines() {
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
        let Some(message) = record.get("message") else {
            continue;
        };
        let role = message.get("role").and_then(Value::as_str);
        if role == Some("user") {
            if is_user_prompt_message(message) {
                prompt_seen += 1;
                if prompt_seen > user_turn_index {
                    break;
                }
                if prompt_seen == user_turn_index {
                    in_turn = true;
                    pending.clear();
                    out.clear();
                    sequence = 0;
                }
            }
            if in_turn {
                ingest_message_tools(message, &mut pending, &mut out, &mut sequence);
            }
        } else if in_turn {
            ingest_message_tools(message, &mut pending, &mut out, &mut sequence);
        }
    }

    flush_pending(&mut pending, &mut out, &mut sequence);
    Ok(out)
}

/// Same as [`list_tool_executions_for_user_turn`] but attaches timestamps from progress events.
pub fn list_tool_executions_for_user_turn_with_time_window(
    session_home: &Path,
    user_turn_index: usize,
    turn_created_at_ms: Option<i64>,
    turn_finished_at_ms: Option<i64>,
) -> Result<Vec<TurnToolRecord>, String> {
    let jsonl_path = gateway_solve_session_persistence_path(session_home);
    let contents = if jsonl_path.is_file() {
        std::fs::read_to_string(&jsonl_path).map_err(|e| format!("read jsonl failed: {e}"))?
    } else {
        String::new()
    };
    let mut out =
        list_tool_executions_for_user_turn_from_jsonl_contents(&contents, user_turn_index)?;

    if turn_created_at_ms.is_some() || turn_finished_at_ms.is_some() {
        enrich_tool_timestamps_from_progress(
            session_home,
            turn_created_at_ms,
            turn_finished_at_ms,
            &mut out,
        )?;
    }

    Ok(out)
}

/// Parse tools from PG `render_session_jsonl`; timestamps from `solve_timing_jsonb.progressEvents`.
pub fn list_tool_executions_for_user_turn_from_jsonl_with_time_window(
    jsonl_contents: &str,
    progress_events: &[ProgressEvent],
    user_turn_index: usize,
    turn_created_at_ms: Option<i64>,
    turn_finished_at_ms: Option<i64>,
) -> Result<Vec<TurnToolRecord>, String> {
    let mut out =
        list_tool_executions_for_user_turn_from_jsonl_contents(jsonl_contents, user_turn_index)?;
    if turn_created_at_ms.is_some() || turn_finished_at_ms.is_some() {
        enrich_tool_timestamps_from_progress_events(
            progress_events,
            turn_created_at_ms,
            turn_finished_at_ms,
            &mut out,
        );
    }
    Ok(out)
}

/// Attach `started_at_ms` / `finished_at_ms` from PG progress events (`mcp_tool_started`).
pub fn enrich_tool_timestamps_from_progress_events(
    progress_events: &[ProgressEvent],
    turn_created_at_ms: Option<i64>,
    turn_finished_at_ms: Option<i64>,
    tools: &mut [TurnToolRecord],
) {
    if tools.is_empty() {
        return;
    }
    let from = turn_created_at_ms.unwrap_or(0);
    let to = turn_finished_at_ms.unwrap_or(i64::MAX);
    let started_ts: Vec<i64> = progress_events
        .iter()
        .filter(|ev| ev.kind == "mcp_tool_started" && ev.ts_ms >= from && ev.ts_ms <= to)
        .map(|ev| ev.ts_ms)
        .collect();
    apply_tool_timestamp_windows(started_ts, turn_created_at_ms, turn_finished_at_ms, tools);
}

fn flush_pending(
    pending: &mut Vec<PendingTool>,
    out: &mut Vec<TurnToolRecord>,
    sequence: &mut u32,
) {
    for p in pending.drain(..) {
        *sequence += 1;
        out.push(TurnToolRecord {
            tool_use_id: p.tool_use_id,
            name: p.name,
            input: p.input,
            output: None,
            is_error: None,
            sequence: *sequence,
            started_at_ms: None,
            finished_at_ms: None,
        });
    }
}

/// User prompt line (aligns with `gateway_turns` index), not tool-result carrier messages.
fn is_user_prompt_message(message: &Value) -> bool {
    let Some(blocks) = message.get("blocks").and_then(Value::as_array) else {
        return false;
    };
    blocks.iter().any(|b| {
        b.get("type").and_then(Value::as_str) == Some("text")
            && b.get("text")
                .and_then(Value::as_str)
                .is_some_and(|t| !t.trim().is_empty())
    })
}

fn message_blocks(message: &Value) -> Vec<&Value> {
    if let Some(blocks) = message.get("blocks").and_then(Value::as_array) {
        return blocks.iter().collect();
    }
    if let Some(content) = message.get("content") {
        if let Some(arr) = content.as_array() {
            return arr.iter().collect();
        }
        return vec![content];
    }
    Vec::new()
}

fn ingest_message_tools(
    message: &Value,
    pending: &mut Vec<PendingTool>,
    out: &mut Vec<TurnToolRecord>,
    sequence: &mut u32,
) {
    for block in message_blocks(message) {
        let Some(block_type) = block.get("type").and_then(Value::as_str) else {
            continue;
        };
        match block_type {
            "tool_use" => {
                let tool_use_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let input = parse_tool_input_block(block);
                pending.push(PendingTool {
                    tool_use_id,
                    name,
                    input,
                });
            }
            "tool_result" => {
                let tool_use_id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let output = extract_tool_output(block);
                let is_error = block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .filter(|&b| b);
                if let Some(pos) = pending.iter().position(|p| p.tool_use_id == tool_use_id) {
                    let p = pending.remove(pos);
                    *sequence += 1;
                    out.push(TurnToolRecord {
                        tool_use_id: p.tool_use_id,
                        name: p.name,
                        input: p.input,
                        output,
                        is_error,
                        sequence: *sequence,
                        started_at_ms: None,
                        finished_at_ms: None,
                    });
                } else {
                    *sequence += 1;
                    out.push(TurnToolRecord {
                        tool_use_id,
                        name: String::new(),
                        input: json!({}),
                        output,
                        is_error,
                        sequence: *sequence,
                        started_at_ms: None,
                        finished_at_ms: None,
                    });
                }
            }
            _ => {}
        }
    }
}

fn parse_tool_input_block(block: &Value) -> Value {
    let Some(input) = block.get("input") else {
        return json!({});
    };
    if let Some(s) = input.as_str() {
        return parse_tool_input(s);
    }
    input.clone()
}

fn parse_tool_input(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| json!({ "raw": raw }))
}

/// Extract tool output from persisted `tool_result` block (`output` or legacy `content`).
fn extract_tool_output(block: &Value) -> Option<String> {
    if let Some(s) = block.get("output").and_then(Value::as_str) {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    let content = block.get("content")?;
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            if let Some(t) = item.get("text").and_then(Value::as_str) {
                parts.push(t.to_string());
            } else if let Some(s) = item.as_str() {
                parts.push(s.to_string());
            } else {
                parts.push(item.to_string());
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    if content.is_object() || content.is_array() {
        return Some(content.to_string());
    }
    None
}

#[derive(Debug, Deserialize)]
struct ProgressEventLine {
    #[serde(rename = "tsMs")]
    ts_ms: i64,
    kind: String,
}

fn enrich_tool_timestamps_from_progress(
    session_home: &Path,
    turn_created_at_ms: Option<i64>,
    turn_finished_at_ms: Option<i64>,
    tools: &mut [TurnToolRecord],
) -> Result<(), String> {
    if tools.is_empty() {
        return Ok(());
    }
    let path = progress_events_path(session_home);
    if !path.is_file() {
        return Ok(());
    }
    let contents =
        std::fs::read_to_string(&path).map_err(|e| format!("read progress-events failed: {e}"))?;
    let from = turn_created_at_ms.unwrap_or(0);
    let to = turn_finished_at_ms.unwrap_or(i64::MAX);

    let mut started_ts: Vec<i64> = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(ev) = serde_json::from_str::<ProgressEventLine>(line) else {
            continue;
        };
        if ev.kind != "mcp_tool_started" {
            continue;
        }
        if ev.ts_ms < from || ev.ts_ms > to {
            continue;
        }
        started_ts.push(ev.ts_ms);
    }

    apply_tool_timestamp_windows(started_ts, turn_created_at_ms, turn_finished_at_ms, tools);
    Ok(())
}

fn apply_tool_timestamp_windows(
    started_ts: Vec<i64>,
    turn_created_at_ms: Option<i64>,
    turn_finished_at_ms: Option<i64>,
    tools: &mut [TurnToolRecord],
) {
    for (i, tool) in tools.iter_mut().enumerate() {
        if let Some(&ts) = started_ts.get(i) {
            tool.started_at_ms = Some(ts);
            let end = started_ts
                .get(i + 1)
                .copied()
                .or(turn_finished_at_ms)
                .filter(|&t| t >= ts);
            tool.finished_at_ms = end;
        } else if let Some(created) = turn_created_at_ms {
            tool.started_at_ms = Some(created);
            if tool.output.is_some() {
                tool.finished_at_ms = turn_finished_at_ms;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn write_jsonl(dir: &Path, lines: &[&str]) {
        let claw = dir.join(".claw");
        std::fs::create_dir_all(&claw).unwrap();
        let path = claw.join("gateway-solve-session.jsonl");
        let mut f = File::create(path).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
    }

    #[test]
    fn lists_tools_for_second_user_turn() {
        let dir = tempfile::tempdir().unwrap();
        write_jsonl(
            dir.path(),
            &[
                r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"first"}]}}"#,
                r#"{"type":"message","message":{"role":"assistant","blocks":[{"type":"tool_use","id":"t1","name":"bash","input":"{}"}]}}"#,
                r#"{"type":"message","message":{"role":"user","blocks":[{"type":"tool_result","tool_use_id":"t1","tool_name":"bash","output":"ok1","is_error":false}]}}"#,
                r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"second"}]}}"#,
                r#"{"type":"message","message":{"role":"assistant","blocks":[{"type":"tool_use","id":"t2","name":"read","input":"{}"}]}}"#,
                r#"{"type":"message","message":{"role":"user","blocks":[{"type":"tool_result","tool_use_id":"t2","tool_name":"read","output":"ok2","is_error":false}]}}"#,
            ],
        );
        let tools = list_tool_executions_for_user_turn(dir.path(), 2).unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "read");
        assert_eq!(tools[0].output.as_deref(), Some("ok2"));
        assert_eq!(tools[0].sequence, 1);
    }

    #[test]
    fn pending_tools_keep_jsonl_order() {
        let dir = tempfile::tempdir().unwrap();
        write_jsonl(
            dir.path(),
            &[
                r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"q"}]}}"#,
                r#"{"type":"message","message":{"role":"assistant","blocks":[{"type":"tool_use","id":"a","name":"first","input":"{}"},{"type":"tool_use","id":"b","name":"second","input":"{}"}]}}"#,
            ],
        );
        let tools = list_tool_executions_for_user_turn(dir.path(), 1).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "first");
        assert_eq!(tools[1].name, "second");
        assert_eq!(tools[0].sequence, 1);
        assert_eq!(tools[1].sequence, 2);
        assert!(tools[0].output.is_none());
    }

    #[test]
    fn tool_result_array_content() {
        let dir = tempfile::tempdir().unwrap();
        write_jsonl(
            dir.path(),
            &[
                r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"q"}]}}"#,
                r#"{"type":"message","message":{"role":"assistant","blocks":[{"type":"tool_use","id":"t1","name":"mcp_x","input":"{}"}]}}"#,
                r#"{"type":"message","message":{"role":"user","blocks":[{"type":"tool_result","tool_use_id":"t1","tool_name":"mcp_x","output":"line1\nline2","is_error":false}]}}"#,
            ],
        );
        let tools = list_tool_executions_for_user_turn(dir.path(), 1).unwrap();
        assert_eq!(tools[0].output.as_deref(), Some("line1\nline2"));
    }

    #[test]
    fn enrich_timestamps_from_progress_events() {
        let dir = tempfile::tempdir().unwrap();
        write_jsonl(
            dir.path(),
            &[
                r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"q"}]}}"#,
                r#"{"type":"message","message":{"role":"assistant","blocks":[{"type":"tool_use","id":"t1","name":"mcp_a","input":"{}"}]}}"#,
                r#"{"type":"message","message":{"role":"user","blocks":[{"type":"tool_result","tool_use_id":"t1","tool_name":"mcp_a","output":"done","is_error":false}]}}"#,
            ],
        );
        let claw = dir.path().join(".claw");
        std::fs::create_dir_all(&claw).unwrap();
        let progress = claw.join("progress-events.ndjson");
        let mut f = File::create(&progress).unwrap();
        writeln!(
            f,
            r#"{{"tsMs":1000,"kind":"mcp_tool_started","tool":"mcp_a"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"tsMs":500,"kind":"report_progress","message":"x"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"tsMs":2000,"kind":"mcp_tool_started","tool":"mcp_b"}}"#
        )
        .unwrap();

        let tools = list_tool_executions_for_user_turn_with_time_window(
            dir.path(),
            1,
            Some(900),
            Some(2500),
        )
        .unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].started_at_ms, Some(1000));
        assert_eq!(tools[0].finished_at_ms, Some(2000));
    }
}
