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

/// Minimal pass-through for API read path: hide obvious internal tool id strings only.
/// User-facing wording is enforced via system prompt (future: small fast model gate). Author: kejiqing
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

/// Appended to gateway solve system prompt so the model reports user-visible progress. Author: kejiqing
#[must_use]
pub fn gateway_progress_system_section() -> &'static str {
    r"## Gateway user-visible progress (required)

- Call the `report_progress` tool whenever the user-visible phase changes (planning, each todo start/finish, before/after long tool use, completion).
- Set `current_task_desc` to one short **business** sentence the boss can understand (<=80 chars): say **what you are doing on the business task**, not how the system works.
- **Prefer specific progress** tied to the user ask, e.g. 「获取昨日门店销售数据」「核对门店营业额口径」「汇总区域同比」「撰写经营结论要点」. Do **not** default to vague lines like only 「数据查询中」 or 「处理中」 when you can name the actual step.
- Generic fallbacks are OK only when no clearer business step exists yet (e.g. first moment: 「分析计划组织中」).
- Forbidden in `current_task_desc`, `plan_title`, and `todos[].title`: file paths, CSV/JSON/XLSX, workspace directories, database/SQL/MCP/SQLBot names, connection errors, upload prompts, HTTP/API retries, docker/podman, tokens, or apologies for missing data—never explain *why* a tool or file failed.
- Put intermediate reasoning and drafts only in normal assistant messages, not in `report_progress`.
- Update `todos` when the plan or step status changes; keep todo titles business-facing like `current_task_desc`."
}

/// Constraints for final user-visible assistant replies (not only progress). Author: kejiqing
#[must_use]
pub fn gateway_user_communication_section() -> &'static str {
    r"## User-facing replies (required)

- Speak to the store manager/boss in plain business Chinese. Never expose implementation details.
- Do NOT mention: missing local files, CSV/JSON paths, workspace folders, MCP/SQLBot/database connection failures, HTTP errors, retries, containers, or asking the user to upload data files unless the product UI explicitly supports uploads.
- If data or tools are unavailable, give a brief neutral outcome only (e.g. 「暂时无法完成本次分析，请稍后再试」) without technical reasons or remediation steps aimed at engineers.
- Separate channels: `report_progress` = progress only; final answer = conclusions/recommendations only—no infrastructure narration in either channel."
}

#[must_use]
pub fn report_progress_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: REPORT_PROGRESS_TOOL_NAME.to_string(),
        description: Some(
            "Report user-visible task progress: one short business sentence naming the current step (e.g. 获取昨日门店销售数据); not generic 处理中/数据查询中 unless unavoidable; no file/DB/MCP/errors.".to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "current_task_desc": {
                    "type": "string",
                    "description": "One boss-facing progress sentence (<=80 chars), specific to the task, e.g. 获取昨日门店销售数据、汇总营业额同比；avoid vague 数据查询中 unless no clearer step"
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
