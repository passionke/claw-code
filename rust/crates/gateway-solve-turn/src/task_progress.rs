//! Gateway user-visible progress (`report_progress` tool → `.claw/task-progress.json`). Author: kejiqing

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use api::ToolDefinition;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const REPORT_PROGRESS_TOOL_NAME: &str = "report_progress";

const PROGRESS_VERSION: u32 = 1;
const MAX_CURRENT_TASK_DESC_CHARS: usize = 80;
const MAX_HISTORY_LINES: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgressTodo {
    pub id: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskProgressFile {
    pub version: u32,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "currentTaskDesc")]
    pub current_task_desc: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "planTitle")]
    pub plan_title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub todos: Vec<TaskProgressTodo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "currentTodoId")]
    pub current_todo_id: Option<String>,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReportProgressInput {
    pub current_task_desc: String,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub plan_title: Option<String>,
    #[serde(default)]
    pub todos: Option<Vec<TaskProgressTodo>>,
    #[serde(default)]
    pub current_todo_id: Option<String>,
}

#[must_use]
pub fn task_progress_json_path(session_home: &Path) -> PathBuf {
    session_home.join(".claw").join("task-progress.json")
}

#[must_use]
pub fn task_progress_history_path(session_home: &Path) -> PathBuf {
    session_home.join(".claw").join("task-progress.ndjson")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create progress dir failed: {e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents).map_err(|e| format!("write progress temp failed: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename progress file failed: {e}"))?;
    Ok(())
}

/// Minimal pass-through for API read path: hide obvious internal tool id strings only. Author: kejiqing
#[must_use]
pub fn sanitize_current_task_desc(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("mcp__")
        || lower.contains("sqlbot")
        || lower.contains("mcp/")
        || trimmed.contains("mcp__")
    {
        return "工具调用中".to_string();
    }
    let mut out = trimmed.to_string();
    if out.chars().count() > MAX_CURRENT_TASK_DESC_CHARS {
        out = out.chars().take(MAX_CURRENT_TASK_DESC_CHARS).collect();
    }
    out
}

pub fn reset_task_progress(session_home: &Path, session_id: &str) -> Result<(), String> {
    let progress = TaskProgressFile {
        version: PROGRESS_VERSION,
        session_id: session_id.to_string(),
        current_task_desc: String::new(),
        phase: "starting".to_string(),
        plan_title: None,
        todos: Vec::new(),
        current_todo_id: None,
        updated_at_ms: now_ms(),
    };
    write_task_progress(session_home, &progress)
}

pub fn write_task_progress(session_home: &Path, progress: &TaskProgressFile) -> Result<(), String> {
    let path = task_progress_json_path(session_home);
    let bytes = serde_json::to_vec_pretty(progress)
        .map_err(|e| format!("serialize task progress failed: {e}"))?;
    write_atomic(&path, &bytes)?;
    append_progress_history(session_home, progress)?;
    Ok(())
}

fn append_progress_history(session_home: &Path, progress: &TaskProgressFile) -> Result<(), String> {
    let path = task_progress_history_path(session_home);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create history dir failed: {e}"))?;
    }
    let line = serde_json::to_string(progress)
        .map_err(|e| format!("serialize history line failed: {e}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open progress history failed: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("append progress history failed: {e}"))?;
    trim_history_file(&path)?;
    Ok(())
}

fn trim_history_file(path: &Path) -> Result<(), String> {
    let contents = fs::read_to_string(path).map_err(|e| format!("read history failed: {e}"))?;
    let lines: Vec<&str> = contents.lines().collect();
    if lines.len() <= MAX_HISTORY_LINES {
        return Ok(());
    }
    let tail = lines[lines.len() - MAX_HISTORY_LINES..].join("\n");
    fs::write(path, format!("{tail}\n")).map_err(|e| format!("trim history failed: {e}"))?;
    Ok(())
}

#[must_use]
pub fn read_task_progress(session_home: &Path) -> Option<TaskProgressFile> {
    let path = task_progress_json_path(session_home);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn read_progress_history(
    session_home: &Path,
    limit: usize,
) -> Result<Vec<TaskProgressFile>, String> {
    let path = task_progress_history_path(session_home);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path).map_err(|e| format!("read history failed: {e}"))?;
    let mut out = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<TaskProgressFile>(line) {
            out.push(entry);
        }
    }
    if out.len() > limit {
        out = out.split_off(out.len() - limit);
    }
    Ok(out)
}

pub fn truncate_progress_history(session_home: &Path) -> Result<(), String> {
    let path = task_progress_history_path(session_home);
    if path.is_file() {
        fs::remove_file(path).map_err(|e| format!("remove history failed: {e}"))?;
    }
    Ok(())
}

pub fn run_report_progress(
    session_home: &Path,
    session_id: &str,
    input: &Value,
) -> Result<String, String> {
    let parsed: ReportProgressInput = serde_json::from_value(input.clone())
        .map_err(|e| format!("invalid report_progress input: {e}"))?;
    let mut desc = parsed.current_task_desc.trim().to_string();
    if desc.is_empty() {
        return Err("current_task_desc is required and cannot be empty".to_string());
    }
    if desc.chars().count() > MAX_CURRENT_TASK_DESC_CHARS {
        desc = desc.chars().take(MAX_CURRENT_TASK_DESC_CHARS).collect();
    }
    let phase = parsed
        .phase
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "executing_todo".to_string());
    let progress = TaskProgressFile {
        version: PROGRESS_VERSION,
        session_id: session_id.to_string(),
        current_task_desc: desc,
        phase,
        plan_title: parsed.plan_title.filter(|s| !s.trim().is_empty()),
        todos: parsed.todos.unwrap_or_default(),
        current_todo_id: parsed.current_todo_id.filter(|s| !s.trim().is_empty()),
        updated_at_ms: now_ms(),
    };
    write_task_progress(session_home, &progress)?;
    Ok(json!({ "ok": true, "updatedAtMs": progress.updated_at_ms }).to_string())
}

#[must_use]
pub fn report_progress_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: REPORT_PROGRESS_TOOL_NAME.to_string(),
        description: Some(
            "Update task progress shown in the gateway UI (writes `.claw/task-progress.json`)."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "current_task_desc": {
                    "type": "string",
                    "description": "Short user-visible status (<=80 chars)"
                },
                "phase": {
                    "type": "string",
                    "description": "planning | planned | executing_todo | done | failed"
                },
                "plan_title": { "type": "string" },
                "todos": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "title": { "type": "string" },
                            "status": { "type": "string", "description": "pending | in_progress | done | skipped" }
                        },
                        "required": ["id", "title", "status"]
                    }
                },
                "current_todo_id": { "type": "string" }
            },
            "required": ["current_task_desc"],
            "additionalProperties": false
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn sanitize_strips_sqlbot_pattern() {
        assert_eq!(
            sanitize_current_task_desc("calling mcp__sqlbot__query"),
            "工具调用中"
        );
    }

    #[test]
    fn report_progress_writes_desc_as_given() {
        let dir = std::env::temp_dir().join(format!("claw-progress-pass-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let input = json!({
            "current_task_desc": "正在汇总门店营业额",
            "phase": "executing_todo"
        });
        run_report_progress(&dir, "sess-pass", &input).unwrap();
        let p = read_task_progress(&dir).unwrap();
        assert_eq!(p.current_task_desc, "正在汇总门店营业额");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_progress_writes_file() {
        let dir = std::env::temp_dir().join(format!("claw-progress-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let input = json!({
            "current_task_desc": "分析计划组织中",
            "phase": "planning"
        });
        run_report_progress(&dir, "sess-1", &input).unwrap();
        let p = read_task_progress(&dir).unwrap();
        assert_eq!(p.current_task_desc, "分析计划组织中");
        assert_eq!(p.phase, "planning");
        let _ = fs::remove_dir_all(&dir);
    }
}
