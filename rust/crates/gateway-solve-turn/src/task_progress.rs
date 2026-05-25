//! Gateway user-visible progress (`report_progress` tool → `.claw/task-progress.json` + `.claw/progress-events.ndjson`). Author: kejiqing

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use api::ToolDefinition;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::entity_labels::{
    entity_labels_for_progress, substitute_entity_ids_in_text, EntityLabelMap,
};

pub const REPORT_PROGRESS_TOOL_NAME: &str = "report_progress";
/// `progressHistory` kind when the model calls [`REPORT_PROGRESS_TOOL_NAME`]. Author: kejiqing
pub const REPORT_PROGRESS_EVENT_KIND: &str = "report_progress";

const PROGRESS_VERSION: u32 = 1;
pub const DEFAULT_PROGRESS_MESSAGE_MAX_CHARS: usize = 80;
const PROGRESS_MESSAGE_TRUNCATE_SUFFIX: &str = "...";
const MAX_EVENT_HISTORY_LINES: usize = 200;

/// Max Unicode scalars per progress line (`progressHistory` / `currentTaskDesc`). Env: `CLAW_PROGRESS_MESSAGE_MAX_CHARS`. Author: kejiqing
#[must_use]
pub fn progress_message_max_chars() -> usize {
    std::env::var("CLAW_PROGRESS_MESSAGE_MAX_CHARS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_PROGRESS_MESSAGE_MAX_CHARS)
}

fn truncate_to_max_chars(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let suffix_len = PROGRESS_MESSAGE_TRUNCATE_SUFFIX.chars().count();
    if max <= suffix_len {
        return s.chars().take(max).collect();
    }
    let take = max - suffix_len;
    format!(
        "{}{}",
        s.chars().take(take).collect::<String>(),
        PROGRESS_MESSAGE_TRUNCATE_SUFFIX
    )
}

/// Append-only factual timeline entry (not todo snapshots). Author: kejiqing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEvent {
    pub kind: String,
    pub message: String,
    #[serde(rename = "tsMs")]
    pub ts_ms: i64,
}

/// NL query / analysis MCP tools only (excludes gateway `SQLBot` preflight: start, datasource list, tables).
/// Author: kejiqing
#[must_use]
pub fn is_mcp_query_progress_tool(tool_name: &str) -> bool {
    tool_name.contains("mcp_question")
}

/// Whether tool execution should append to `.claw/progress-events.ndjson`.
/// Whitelist: `mcp_question*` runtime tools and legacy `MCP` wrapper when `tool` is query-class. Author: kejiqing
#[must_use]
pub fn should_emit_tool_progress_event(
    tool_name: &str,
    is_registered_runtime_mcp: bool,
    mcp_args: Option<&Value>,
) -> bool {
    if tool_name == "MCP" {
        let inner = mcp_args
            .and_then(|a| a.get("tool"))
            .and_then(Value::as_str)
            .unwrap_or("");
        return is_mcp_query_progress_tool(inner);
    }
    is_registered_runtime_mcp && is_mcp_query_progress_tool(tool_name)
}

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

#[must_use]
pub fn progress_events_path(session_home: &Path) -> PathBuf {
    session_home.join(".claw").join("progress-events.ndjson")
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
    truncate_to_max_chars(trimmed, progress_message_max_chars())
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
    write_atomic(&path, &bytes)
}

fn append_ndjson_line(path: &Path, line: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create events dir failed: {e}"))?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open events file failed: {e}"))?;
    writeln!(file, "{line}").map_err(|e| format!("append events failed: {e}"))?;
    trim_ndjson_file(path)?;
    Ok(())
}

pub fn append_progress_event(session_home: &Path, event: &ProgressEvent) -> Result<(), String> {
    let path = progress_events_path(session_home);
    let line = serde_json::to_string(event)
        .map_err(|e| format!("serialize progress event failed: {e}"))?;
    append_ndjson_line(&path, &line)
}

fn trim_ndjson_file(path: &Path) -> Result<(), String> {
    let contents = fs::read_to_string(path).map_err(|e| format!("read history failed: {e}"))?;
    let lines: Vec<&str> = contents.lines().collect();
    if lines.len() <= MAX_EVENT_HISTORY_LINES {
        return Ok(());
    }
    let tail = lines[lines.len() - MAX_EVENT_HISTORY_LINES..].join("\n");
    fs::write(path, format!("{tail}\n")).map_err(|e| format!("trim events failed: {e}"))?;
    Ok(())
}

/// User-visible line from MCP tool args (`question`, `query`, …), with id→name substitution. Author: kejiqing
#[must_use]
pub fn progress_message_from_mcp_input(
    session_home: &Path,
    extra_session: Option<&Value>,
    args: &Value,
) -> String {
    let labels = entity_labels_for_progress(session_home, extra_session);
    progress_message_from_mcp_input_with_labels(args, &labels)
}

#[must_use]
pub fn progress_message_from_mcp_input_with_labels(
    args: &Value,
    labels: &EntityLabelMap,
) -> String {
    for key in ["question", "query", "prompt", "message", "text"] {
        if let Some(s) = args.get(key).and_then(Value::as_str) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                let substituted = substitute_entity_ids_in_text(trimmed, labels);
                let out = sanitize_current_task_desc(&substituted);
                if !out.is_empty() {
                    return out;
                }
            }
        }
    }
    "数据查询中".to_string()
}

pub fn record_mcp_tool_started(
    session_home: &Path,
    session_id: &str,
    extra_session: Option<&Value>,
    args: &Value,
) -> Result<(), String> {
    let message = progress_message_from_mcp_input(session_home, extra_session, args);
    patch_current_task_desc(session_home, session_id, &message, "executing")?;
    append_progress_event(
        session_home,
        &ProgressEvent {
            kind: "mcp_tool_started".to_string(),
            message,
            ts_ms: now_ms(),
        },
    )
}

/// Append model-reported user-visible status to `.claw/progress-events.ndjson`. Author: kejiqing
pub fn record_report_progress_event(
    session_home: &Path,
    message: &str,
    ts_ms: i64,
) -> Result<(), String> {
    append_progress_event(
        session_home,
        &ProgressEvent {
            kind: REPORT_PROGRESS_EVENT_KIND.to_string(),
            message: message.to_string(),
            ts_ms,
        },
    )
}

fn patch_current_task_desc(
    session_home: &Path,
    session_id: &str,
    desc: &str,
    phase: &str,
) -> Result<(), String> {
    let Some(mut progress) = read_task_progress(session_home) else {
        // Avoid clobbering plan/todos when orchestrator progress is still being written.
        return Ok(());
    };
    progress.session_id = session_id.to_string();
    progress.current_task_desc = desc.to_string();
    progress.phase = phase.to_string();
    progress.updated_at_ms = now_ms();
    write_task_progress(session_home, &progress)
}

#[must_use]
pub fn read_task_progress(session_home: &Path) -> Option<TaskProgressFile> {
    let path = task_progress_json_path(session_home);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn read_progress_events(
    session_home: &Path,
    limit: usize,
) -> Result<Vec<ProgressEvent>, String> {
    let path = progress_events_path(session_home);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path).map_err(|e| format!("read events failed: {e}"))?;
    let mut out = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<ProgressEvent>(line) {
            out.push(entry);
        }
    }
    if out.len() > limit {
        out = out.split_off(out.len() - limit);
    }
    Ok(out)
}

/// Legacy `task-progress.ndjson` snapshots; prefer [`read_progress_events`]. Author: kejiqing
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

pub fn truncate_progress_events(session_home: &Path) -> Result<(), String> {
    let path = progress_events_path(session_home);
    if path.is_file() {
        fs::remove_file(path).map_err(|e| format!("remove progress events failed: {e}"))?;
    }
    Ok(())
}

pub fn truncate_progress_history(session_home: &Path) -> Result<(), String> {
    let _ = truncate_progress_events(session_home);
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
    desc = truncate_to_max_chars(&desc, progress_message_max_chars());
    let phase = parsed
        .phase
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "executing_todo".to_string());
    let existing = read_task_progress(session_home);
    let todos = match &parsed.todos {
        Some(items) if !items.is_empty() => items.clone(),
        _ => existing
            .as_ref()
            .map(|p| p.todos.clone())
            .unwrap_or_default(),
    };
    let plan_title = parsed
        .plan_title
        .filter(|s| !s.trim().is_empty())
        .or_else(|| existing.as_ref().and_then(|p| p.plan_title.clone()));
    let progress = TaskProgressFile {
        version: PROGRESS_VERSION,
        session_id: session_id.to_string(),
        current_task_desc: desc,
        phase,
        plan_title,
        todos,
        current_todo_id: parsed
            .current_todo_id
            .filter(|s| !s.trim().is_empty())
            .or_else(|| existing.and_then(|p| p.current_todo_id)),
        updated_at_ms: now_ms(),
    };
    write_task_progress(session_home, &progress)?;
    record_report_progress_event(
        session_home,
        &progress.current_task_desc,
        progress.updated_at_ms,
    )?;
    Ok(json!({ "ok": true, "updatedAtMs": progress.updated_at_ms }).to_string())
}

#[must_use]
pub fn report_progress_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: REPORT_PROGRESS_TOOL_NAME.to_string(),
        description: Some(
            "Update task progress shown in the gateway UI (writes `.claw/task-progress.json` and appends `.claw/progress-events.ndjson`)."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "current_task_desc": {
                    "type": "string",
                    "description": "Short user-visible status (length capped by CLAW_PROGRESS_MESSAGE_MAX_CHARS, default 80)"
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
        let events = read_progress_events(&dir, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, REPORT_PROGRESS_EVENT_KIND);
        assert_eq!(events[0].message, "正在汇总门店营业额");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncate_appends_ellipsis_when_over_limit() {
        let long = "查".repeat(100);
        let out = truncate_to_max_chars(&long, 80);
        assert!(out.ends_with("..."));
        assert_eq!(out.chars().count(), 80);
    }

    #[test]
    fn mcp_progress_history_only_records_started() {
        let dir = std::env::temp_dir().join(format!("claw-mcp-done-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let args = json!({ "question": "分析门店营业额趋势" });
        record_mcp_tool_started(&dir, "sess-dup", None, &args).unwrap();
        let events = read_progress_events(&dir, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "mcp_tool_started");
        assert_eq!(events[0].message, "分析门店营业额趋势");
        assert!(
            !events
                .iter()
                .any(|e| { e.kind == "mcp_tool_completed" || e.kind == "mcp_tool_failed" }),
            "MCP finish should not append completed/failed progress lines"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn whitelist_only_mcp_question_tools() {
        assert!(should_emit_tool_progress_event(
            "mcp__sqlbot-streamable__mcp_question_then_analysis",
            true,
            None,
        ));
        assert!(should_emit_tool_progress_event(
            "mcp__sqlbot-streamable__mcp_question",
            true,
            None,
        ));
        assert!(!should_emit_tool_progress_event(
            "mcp__sqlbot-streamable__mcp_start",
            true,
            None,
        ));
        assert!(!should_emit_tool_progress_event(
            "mcp__sqlbot-streamable__mcp_datasource_list",
            true,
            None,
        ));
        assert!(!should_emit_tool_progress_event(
            "mcp__sqlbot-streamable__mcp_datasource_tables",
            true,
            None,
        ));
        assert!(!should_emit_tool_progress_event("Bash", false, None));
        assert!(!should_emit_tool_progress_event("Read", false, None));
        let query_wrapper = json!({ "server": "sqlbot", "tool": "mcp_question", "arguments": {} });
        assert!(should_emit_tool_progress_event(
            "MCP",
            false,
            Some(&query_wrapper)
        ));
        let preflight_wrapper = json!({ "server": "sqlbot", "tool": "mcp_start", "arguments": {} });
        assert!(!should_emit_tool_progress_event(
            "MCP",
            false,
            Some(&preflight_wrapper)
        ));
    }

    #[test]
    fn progress_message_substitutes_store_id_when_cached() {
        let dir = std::env::temp_dir().join(format!("claw-labels-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".claw")).unwrap();
        let labels_path = dir.join(".claw/entity-labels.json");
        fs::write(
            &labels_path,
            r#"{"stores":{"S20241007172800004204":"外滩店"},"orgs":{}}"#,
        )
        .unwrap();
        let args = json!({ "question": "统计门店 S20241007172800004204 营业额" });
        let msg = progress_message_from_mcp_input(&dir, None, &args);
        assert!(msg.contains("外滩店"));
        assert!(!msg.contains("S20241007172800004204"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_started_appends_progress_event_not_on_bash() {
        let dir = std::env::temp_dir().join(format!("claw-events-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let args = json!({ "question": "统计门店营业额" });
        record_mcp_tool_started(&dir, "sess-ev", None, &args).unwrap();
        let events = read_progress_events(&dir, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "mcp_tool_started");
        assert_eq!(events[0].message, "统计门店营业额");
        assert!(!should_emit_tool_progress_event("Glob", false, None));
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
        let events = read_progress_events(&dir, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "report_progress");
        assert_eq!(events[0].message, "分析计划组织中");
        let _ = fs::remove_dir_all(&dir);
    }
}
