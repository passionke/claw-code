//! Task `currentTaskDesc` resolution and gateway queue stats. Author: kejiqing

use std::collections::HashMap;
use std::path::Path;

use gateway_solve_turn::{
    read_task_progress, sanitize_current_task_desc, REPORT_PROGRESS_TOOL_NAME,
};
use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayQueueSnapshot {
    pub gateway_tasks_queued: usize,
    pub gateway_tasks_running: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_slots_idle: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_slots_leased: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool_size: Option<usize>,
}

#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn count_gateway_tasks(tasks: &HashMap<String, TaskStatusRow>) -> GatewayQueueSnapshot {
    let mut queued = 0usize;
    let mut running = 0usize;
    for row in tasks.values() {
        match row.status.as_str() {
            "queued" => queued += 1,
            "running" => running += 1,
            _ => {}
        }
    }
    GatewayQueueSnapshot {
        gateway_tasks_queued: queued,
        gateway_tasks_running: running,
        pool_slots_idle: None,
        pool_slots_leased: None,
        pool_size: None,
    }
}

/// Minimal task row for queue counting (avoids coupling to binary `TaskRecord`).
#[derive(Debug, Clone)]
pub struct TaskStatusRow {
    pub status: String,
}

#[must_use]
pub fn format_queued_desc(snapshot: &GatewayQueueSnapshot) -> String {
    format!(
        "排队中（{} 个等待，{} 个执行中）",
        snapshot.gateway_tasks_queued, snapshot.gateway_tasks_running
    )
}

#[must_use]
pub fn terminal_fallback_desc(status: &str) -> Option<String> {
    match status {
        "succeeded" => Some("分析完成".to_string()),
        "failed" => Some("任务失败".to_string()),
        "cancelled" => Some("任务已取消".to_string()),
        _ => None,
    }
}

/// Resolve user-visible progress for a task from on-disk progress file and task status.
#[must_use]
pub fn resolve_current_task_desc(
    status: &str,
    session_home: Option<&Path>,
    queue: &GatewayQueueSnapshot,
    trace_suggests_tool: bool,
) -> Option<String> {
    if let Some(home) = session_home {
        if let Some(progress) = read_task_progress(home) {
            let desc = sanitize_current_task_desc(&progress.current_task_desc);
            if !desc.is_empty() {
                return Some(desc);
            }
        }
    }

    if status == "queued" {
        return Some(format_queued_desc(queue));
    }

    if matches!(status, "succeeded" | "failed" | "cancelled") {
        if let Some(home) = session_home {
            if let Some(progress) = read_task_progress(home) {
                let desc = sanitize_current_task_desc(&progress.current_task_desc);
                if !desc.is_empty() {
                    return Some(desc);
                }
            }
        }
        return terminal_fallback_desc(status);
    }

    if status == "running" {
        if trace_suggests_tool {
            return Some("工具调用中".to_string());
        }
        return Some("处理中".to_string());
    }

    None
}

pub fn ensure_report_progress_in_allowed_tools(tools: &mut Vec<String>) {
    if tools.is_empty() {
        return;
    }
    if tools.iter().any(|t| t == REPORT_PROGRESS_TOOL_NAME) {
        return;
    }
    tools.push(REPORT_PROGRESS_TOOL_NAME.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_uses_queue_template() {
        let q = GatewayQueueSnapshot {
            gateway_tasks_queued: 2,
            gateway_tasks_running: 1,
            ..Default::default()
        };
        let desc = resolve_current_task_desc("queued", None, &q, false).unwrap();
        assert!(desc.contains('2'));
        assert!(desc.contains("排队"));
    }

    #[test]
    fn running_fallback_processing() {
        let q = GatewayQueueSnapshot::default();
        let desc = resolve_current_task_desc("running", None, &q, false).unwrap();
        assert_eq!(desc, "处理中");
    }
}
