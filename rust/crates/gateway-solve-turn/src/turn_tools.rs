//! Tool execution records for one user turn (from session jsonl). Author: kejiqing

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::gateway_solve_session_persistence_path;

const DEFAULT_MAX_FIELD_CHARS: usize = 120_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnToolRecord {
    pub tool_use_id: String,
    pub tool_name: String,
    pub input: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub input_truncated: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub output_truncated: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn parse_tool_input(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Object(serde_json::Map::new());
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn truncate_field(s: String, max_chars: usize) -> (String, bool) {
    if s.chars().count() <= max_chars {
        return (s, false);
    }
    let truncated: String = s.chars().take(max_chars).collect();
    (format!("{truncated}\n…(truncated)"), true)
}

/// Next user **prompt** line (not `tool_result` carrier messages).
fn is_user_turn_boundary(msg: &Value) -> bool {
    let Some(blocks) = msg.get("blocks").and_then(Value::as_array) else {
        return false;
    };
    blocks.iter().any(|block| {
        block.get("type").and_then(Value::as_str) == Some("text")
            && block
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|t| !t.trim().is_empty())
    })
}

fn max_field_chars() -> usize {
    std::env::var("CLAW_TURN_TOOLS_MAX_FIELD_CHARS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_FIELD_CHARS)
}

/// List tool_use / tool_result pairs for the `user_turn_index_1based`-th user message in jsonl.
pub fn list_tool_executions_for_user_turn(
    session_home: &Path,
    user_turn_index_1based: usize,
) -> Result<Vec<TurnToolRecord>, String> {
    if user_turn_index_1based == 0 {
        return Err("user_turn_index_1based must be >= 1".to_string());
    }
    let path = gateway_solve_session_persistence_path(session_home);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let contents =
        fs::read_to_string(&path).map_err(|e| format!("read session jsonl failed: {e}"))?;
    let max_chars = max_field_chars();

    let mut user_seen = 0usize;
    let mut capturing = false;
    let mut pending: HashMap<String, (String, Value)> = HashMap::new();
    let mut out = Vec::new();

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
        let role = msg.get("role").and_then(Value::as_str);
        if role == Some("user") && is_user_turn_boundary(msg) {
            if capturing {
                break;
            }
            user_seen += 1;
            if user_seen == user_turn_index_1based {
                capturing = true;
            }
            continue;
        }
        if !capturing {
            continue;
        }
        if role != Some("assistant") && role != Some("user") {
            continue;
        }
        let Some(blocks) = msg.get("blocks").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let Some(id) = block.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(name) = block.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let input_raw = block
                        .get("input")
                        .and_then(Value::as_str)
                        .unwrap_or("{}");
                    pending.insert(
                        id.to_string(),
                        (name.to_string(), parse_tool_input(input_raw)),
                    );
                }
                Some("tool_result") => {
                    let Some(use_id) = block.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    let (tool_name, input) = pending
                        .remove(use_id)
                        .or_else(|| {
                            block.get("tool_name").and_then(Value::as_str).map(|n| {
                                (
                                    n.to_string(),
                                    Value::Object(serde_json::Map::new()),
                                )
                            })
                        })
                        .unwrap_or_else(|| {
                            ("unknown".to_string(), Value::Object(serde_json::Map::new()))
                        });
                    let output_raw = block
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let (output, output_truncated) = truncate_field(output_raw, max_chars);
                    let (input, input_truncated) = match &input {
                        Value::String(s) => {
                            let (t, tr) = truncate_field(s.clone(), max_chars);
                            (Value::String(t), tr)
                        }
                        _ => (input, false),
                    };
                    out.push(TurnToolRecord {
                        tool_use_id: use_id.to_string(),
                        tool_name,
                        input,
                        output: Some(output),
                        is_error: block.get("is_error").and_then(Value::as_bool),
                        input_truncated,
                        output_truncated,
                    });
                }
                _ => {}
            }
        }
    }

    for (tool_use_id, (tool_name, input)) in pending {
        let (input, input_truncated) = match &input {
            Value::String(s) => {
                let (t, tr) = truncate_field(s.clone(), max_chars);
                (Value::String(t), tr)
            }
            _ => (input, false),
        };
        out.push(TurnToolRecord {
            tool_use_id,
            tool_name,
            input,
            output: None,
            is_error: None,
            input_truncated,
            output_truncated: false,
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_session_dir(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "{prefix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ))
    }

    fn write_jsonl(dir: &Path, body: &str) {
        let claw = dir.join(".claw");
        fs::create_dir_all(&claw).unwrap();
        fs::write(claw.join("gateway-solve-session.jsonl"), body).unwrap();
    }

    #[test]
    fn lists_tools_for_second_user_turn_only() {
        let dir = temp_session_dir("claw-turn-tools");
        write_jsonl(
            &dir,
            r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"q1"}]}}
{"type":"message","message":{"role":"assistant","blocks":[{"type":"tool_use","id":"u1","name":"bash","input":"{\"command\":\"ls\"}"}]}}
{"type":"message","message":{"role":"user","blocks":[{"type":"tool_result","tool_use_id":"u1","tool_name":"bash","output":"ok","is_error":false}]}}
{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"q2"}]}}
{"type":"message","message":{"role":"assistant","blocks":[{"type":"tool_use","id":"u2","name":"mcp__sqlbot__q","input":"{\"question\":\"hi\"}"}]}}
{"type":"message","message":{"role":"user","blocks":[{"type":"tool_result","tool_use_id":"u2","tool_name":"mcp__sqlbot__q","output":"err","is_error":true}]}}
"#,
        );
        let all = list_tool_executions_for_user_turn(&dir, 1).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].tool_name, "bash");
        assert_eq!(all[0].output.as_deref(), Some("ok"));

        let turn2 = list_tool_executions_for_user_turn(&dir, 2).unwrap();
        assert_eq!(turn2.len(), 1);
        assert_eq!(turn2[0].tool_name, "mcp__sqlbot__q");
        assert_eq!(turn2[0].is_error, Some(true));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pending_tool_use_without_result_is_included() {
        let dir = temp_session_dir("claw-turn-tools-pending");
        write_jsonl(
            &dir,
            r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"q"}]}}
{"type":"message","message":{"role":"assistant","blocks":[{"type":"tool_use","id":"u9","name":"read_file","input":"{}"}]}}
"#,
        );
        let rows = list_tool_executions_for_user_turn(&dir, 1).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tool_use_id, "u9");
        assert!(rows[0].output.is_none());
        let _ = fs::remove_dir_all(&dir);
    }
}
