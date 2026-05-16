//! `GET /v1/sessions/{session_id}/execution` — progress, queue, trace tail. Author: kejiqing

use std::path::{Path, PathBuf};

use crate::session_merge;
use crate::task_status::GatewayQueueSnapshot;
use gateway_solve_turn::TaskProgressFile;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExecutionResponse {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "dsId")]
    pub ds_id: i64,
    #[serde(rename = "sessionHomeRel")]
    pub session_home_rel: String,
    pub task: SessionExecutionTask,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskProgressFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub progress_history: Vec<TaskProgressFile>,
    pub queue: GatewayQueueSnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace_tail: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExecutionTask {
    #[serde(rename = "taskId")]
    pub task_id: String,
    pub status: String,
    #[serde(rename = "createdAtMs")]
    pub created_at_ms: i64,
    #[serde(rename = "startedAtMs", skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(rename = "finishedAtMs", skip_serializing_if = "Option::is_none")]
    pub finished_at_ms: Option<i64>,
    #[serde(rename = "currentTaskDesc", skip_serializing_if = "Option::is_none")]
    pub current_task_desc: Option<String>,
}

/// Discover NDJSON trace file for a session (pool guest layout vs global trace dir).
#[must_use]
pub fn discover_trace_paths(
    session_home: &Path,
    work_root: &Path,
    session_id: &str,
) -> Vec<PathBuf> {
    let mut paths = vec![
        session_home
            .join("traces")
            .join(format!("{session_id}.ndjson")),
        work_root
            .join("traces")
            .join(format!("{session_id}.ndjson")),
    ];
    if let Ok(raw) = std::env::var("CLAW_TRACE_DIR") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            paths.push(PathBuf::from(trimmed).join(format!("{session_id}.ndjson")));
        }
    }
    paths
}

#[must_use]
pub fn read_trace_tail(paths: &[PathBuf], limit: usize, include_sensitive: bool) -> Vec<Value> {
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        let lines: Vec<&str> = contents.lines().collect();
        let start = lines.len().saturating_sub(limit);
        let mut out = Vec::new();
        for line in &lines[start..] {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(mut v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if sanitize_trace_value(&mut v, include_sensitive) {
                out.push(v);
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

pub fn trace_tail_suggests_tool_call(paths: &[PathBuf]) -> bool {
    let tail = read_trace_tail(paths, 8, false);
    for entry in tail.iter().rev() {
        if entry.get("name").and_then(Value::as_str) == Some("tool_execution_started") {
            return true;
        }
    }
    false
}

fn sanitize_trace_value(v: &mut Value, include_sensitive: bool) -> bool {
    let Some(ty) = v.get("type").and_then(Value::as_str) else {
        return false;
    };
    if !matches!(ty, "session_trace" | "agent_trace") {
        return false;
    }
    if !include_sensitive {
        if let Some(attrs) = v.get_mut("attributes").and_then(Value::as_object_mut) {
            attrs.remove("user_input");
            attrs.remove("assistant_preview");
            attrs.remove("error_preview");
            attrs.remove("tool_name");
        }
    }
    true
}

#[must_use]
pub fn join_session_home(work_root: &Path, session_home_rel: &str) -> PathBuf {
    session_merge::join_session_home_from_rel(work_root, session_home_rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_trace_paths_includes_session_traces() {
        let home = Path::new("/tmp/sess");
        let root = Path::new("/wr");
        let paths = discover_trace_paths(home, root, "abc");
        assert!(paths[0].ends_with("traces/abc.ndjson"));
    }
}
