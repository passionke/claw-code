//! Emit L2 `tool.start` / `tool.result` / `tool.end` from `gateway-solve-session.jsonl`. Author: kejiqing

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};

use gateway_solve_turn::gateway_solve_session_persistence_path;
use serde_json::{json, Value};

use crate::agui::EventTapHub;

#[derive(Debug, Default)]
struct TaskToolTapState {
    jsonl_offset: u64,
    started: HashSet<String>,
    finished: HashSet<String>,
}

#[derive(Clone, Default)]
pub struct ToolTapHub {
    per_task: Arc<Mutex<std::collections::HashMap<String, TaskToolTapState>>>,
}

impl ToolTapHub {
    pub fn clear(&self, task_id: &str) {
        self.per_task.lock().expect("tool tap lock").remove(task_id);
    }

    /// Tail session transcript and push tap lines for new tool invocations.
    pub fn sync_session_jsonl(&self, task_id: &str, session_home: &Path, event_tap: &EventTapHub) {
        let path = gateway_solve_session_persistence_path(session_home);
        if !path.is_file() {
            return;
        }
        let mut file = match File::open(&path) {
            Ok(f) => f,
            Err(_) => return,
        };
        let mut states = self.per_task.lock().expect("tool tap lock");
        let state = states.entry(task_id.to_string()).or_default();
        let offset = state.jsonl_offset;
        if file.seek(SeekFrom::Start(offset)).is_err() {
            return;
        }
        let mut chunk = String::new();
        if file.read_to_string(&mut chunk).is_err() {
            return;
        }
        state.jsonl_offset = offset + chunk.len() as u64;
        drop(states);

        for line in chunk.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(outer) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let message = outer.get("message").cloned().unwrap_or(outer);
            ingest_message(task_id, session_home, event_tap, self, &message);
        }
    }
}

fn ingest_message(
    task_id: &str,
    session_home: &Path,
    event_tap: &EventTapHub,
    hub: &ToolTapHub,
    message: &Value,
) {
    let role = message.get("role").and_then(Value::as_str).unwrap_or("");
    let blocks = message
        .get("blocks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    if role == "assistant" {
        for block in &blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let Some(tool_call_id) = block.get("id").and_then(Value::as_str) else {
                continue;
            };
            let tool_name = block
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let mut states = hub.per_task.lock().expect("tool tap lock");
            let state = states.entry(task_id.to_string()).or_default();
            if !state.started.insert(tool_call_id.to_string()) {
                continue;
            }
            drop(states);
            push_tap(
                event_tap,
                task_id,
                "tool.start",
                json!({
                    "toolCallId": tool_call_id,
                    "toolName": tool_name,
                }),
            );
        }
        return;
    }

    if role != "tool" {
        return;
    }

    for block in &blocks {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let tool_call_id = block
            .get("tool_use_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if tool_call_id.is_empty() {
            continue;
        }
        let tool_name = block
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string();
        let is_error = block
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let output = block.get("output").and_then(Value::as_str).unwrap_or("");

        let mut states = hub.per_task.lock().expect("tool tap lock");
        let state = states.entry(task_id.to_string()).or_default();
        if !state.finished.insert(tool_call_id.to_string()) {
            continue;
        }
        drop(states);

        let (ok, payload_kind, payload, summary, error) =
            build_tool_result_envelope(&tool_name, output, is_error, session_home);

        let mut envelope = json!({
            "type": "tool.result",
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "ok": ok,
            "summary": summary,
            "payloadKind": payload_kind,
            "payload": payload,
        });
        if let Some(err) = error {
            envelope["error"] = json!(err);
        }

        push_tap(event_tap, task_id, "tool.result", envelope);
        push_tap(
            event_tap,
            task_id,
            "tool.end",
            json!({
                "toolCallId": tool_call_id,
                "ok": ok,
            }),
        );
    }
}

fn push_tap(event_tap: &EventTapHub, task_id: &str, event_type: &str, extra: Value) {
    let line = crate::agui::tap_line(task_id, event_type, &extra);
    event_tap.push(task_id, &line);
}

fn build_tool_result_envelope(
    tool_name: &str,
    output: &str,
    is_error: bool,
    session_home: &Path,
) -> (bool, &'static str, Value, String, Option<String>) {
    if is_error {
        let preview: String = output.chars().take(200).collect();
        return (
            false,
            payload_kind_for_tool(tool_name),
            json!({ "raw": output }),
            format!("{tool_name} 失败"),
            Some(preview),
        );
    }

    let parsed: Value = serde_json::from_str(output).unwrap_or_else(|_| json!({ "raw": output }));

    let payload_kind = payload_kind_for_tool(tool_name);
    let summary = summarize_tool_output(tool_name, &parsed, session_home);
    (true, payload_kind, parsed, summary, None)
}

fn payload_kind_for_tool(tool_name: &str) -> &'static str {
    match tool_name {
        "write_file" | "Write" => "file_write",
        "edit_file" | "Edit" => "file_edit",
        "read_file" | "Read" => "file_read",
        "bash" | "Bash" => "bash",
        _ => "generic",
    }
}

fn summarize_tool_output(tool_name: &str, payload: &Value, session_home: &Path) -> String {
    match tool_name {
        "write_file" | "Write" | "edit_file" | "Edit" => {
            let path = payload
                .get("filePath")
                .and_then(Value::as_str)
                .unwrap_or("文件");
            let kind = payload
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("write");
            let lines = payload
                .get("structuredPatch")
                .and_then(Value::as_array)
                .map(|hunks| {
                    hunks
                        .iter()
                        .filter_map(|h| h.get("lines").and_then(Value::as_array))
                        .map(|lines| lines.len())
                        .sum::<usize>()
                })
                .unwrap_or(0);
            let display = display_file_path(path, session_home);
            if kind == "create" {
                format!("创建 {display}（{lines} 行）")
            } else {
                format!("更新 {display}（{lines} 行变更）")
            }
        }
        "read_file" | "Read" => {
            let path = payload
                .get("file")
                .and_then(|f| f.get("filePath"))
                .and_then(Value::as_str)
                .or_else(|| payload.get("filePath").and_then(Value::as_str))
                .unwrap_or("文件");
            format!("读取 {}", display_file_path(path, session_home))
        }
        "bash" | "Bash" => {
            let exit = payload.get("exitCode").and_then(Value::as_i64);
            match exit {
                Some(0) | None => "命令执行完成".to_string(),
                Some(code) => format!("命令退出码 {code}"),
            }
        }
        _ => format!("{tool_name} 完成"),
    }
}

/// Prefer a short path in UI when worker uses `/claw_host_root/…`.
fn display_file_path(path: &str, session_home: &Path) -> String {
    let trimmed = path.trim();
    if let Some(rest) = trimmed.strip_prefix("/claw_host_root/") {
        return rest.to_string();
    }
    if let Ok(rel) = std::path::Path::new(trimmed).strip_prefix(session_home) {
        return rel.display().to_string();
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn sync_emits_write_file_tool_result_with_patch() {
        let dir = std::env::temp_dir().join(format!("claw-tool-tap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claw")).unwrap();
        let path = gateway_solve_session_persistence_path(&dir);
        let output = json!({
            "type": "create",
            "filePath": "/claw_host_root/pi_power.awk",
            "content": "#!/usr/bin/awk -f\n",
            "structuredPatch": [{
                "oldStart": 1, "oldLines": 0, "newStart": 1, "newLines": 2,
                "lines": ["+#!/usr/bin/awk -f", "+BEGIN {}"]
            }],
            "originalFile": null,
            "gitDiff": null
        });
        let tool_msg = json!({
            "role": "tool",
            "blocks": [{
                "type": "tool_result",
                "tool_use_id": "call_test_1",
                "tool_name": "write_file",
                "is_error": false,
                "output": output.to_string()
            }]
        });
        let assistant_msg = json!({
            "role": "assistant",
            "blocks": [{
                "type": "tool_use",
                "id": "call_test_1",
                "name": "write_file",
                "input": "{}"
            }]
        });
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "{}", json!({"message": assistant_msg})).unwrap();
        writeln!(f, "{}", json!({"message": tool_msg})).unwrap();

        let hub = ToolTapHub::default();
        let tap = EventTapHub::default();
        hub.sync_session_jsonl("task-1", &dir, &tap);
        let lines = tap.snapshot("task-1");
        assert!(
            lines.iter().any(|l| l.contains("tool.start")),
            "lines: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("tool.result") && l.contains("structuredPatch")),
            "lines: {lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("tool.end")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn display_file_path_strips_claw_host_root() {
        let dir = Path::new("/tmp/ws");
        assert_eq!(
            display_file_path("/claw_host_root/pi_power.awk", dir),
            "pi_power.awk"
        );
    }
}
