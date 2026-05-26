//! Code-driven task-progress updates for multi-agent orchestration. Author: kejiqing

use std::path::Path;

use crate::multi_agent::plan::AnalysisPlan;
use crate::task_progress::{
    read_task_progress, record_report_progress_event, write_task_progress, TaskProgressFile,
    TaskProgressTodo,
};

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn base_progress(session_home: &Path, session_id: &str) -> TaskProgressFile {
    read_task_progress(session_home).unwrap_or(TaskProgressFile {
        version: 1,
        session_id: session_id.to_string(),
        current_task_desc: String::new(),
        phase: String::from("starting"),
        plan_title: None,
        todos: Vec::new(),
        current_todo_id: None,
        updated_at_ms: now_ms(),
    })
}

fn write_progress(session_home: &Path, mut progress: TaskProgressFile) -> Result<(), String> {
    progress.updated_at_ms = now_ms();
    let desc = progress.current_task_desc.clone();
    let ts = progress.updated_at_ms;
    write_task_progress(session_home, &progress)?;
    let _ = record_report_progress_event(session_home, &desc, ts);
    Ok(())
}

fn set_todo_status(todos: &mut [TaskProgressTodo], todo_id: &str, status: &str) {
    for todo in todos.iter_mut() {
        if todo.id == todo_id {
            todo.status = status.to_string();
        }
    }
}

fn done_count(todos: &[TaskProgressTodo]) -> usize {
    todos
        .iter()
        .filter(|t| t.status == "done" || t.status == "skipped")
        .count()
}

/// Publish plan outline to task-progress (todos all pending).
pub fn publish_plan(
    session_home: &Path,
    session_id: &str,
    plan: &AnalysisPlan,
) -> Result<(), String> {
    let mut progress = base_progress(session_home, session_id);
    progress.session_id = session_id.to_string();
    progress.plan_title = Some(plan.plan_title.clone());
    progress.todos = plan.progress_todos_pending();
    progress.phase = String::from("planned");
    progress.current_task_desc = String::from("分析框架已生成");
    progress.current_todo_id = None;
    write_progress(session_home, progress)
}

/// Mark one todo in progress when its MCP query starts.
pub fn on_query_started(
    session_home: &Path,
    session_id: &str,
    todo_id: &str,
    title: &str,
) -> Result<(), String> {
    let mut progress = base_progress(session_home, session_id);
    set_todo_status(&mut progress.todos, todo_id, "in_progress");
    progress.phase = String::from("executing_todo");
    progress.current_todo_id = Some(todo_id.to_string());
    let total = progress.todos.len();
    let done = done_count(&progress.todos);
    progress.current_task_desc = format!("正在查询：{title}（{done}/{total}）");
    write_progress(session_home, progress)
}

/// Mark todo done/skipped after MCP query finishes.
pub fn on_query_finished(
    session_home: &Path,
    session_id: &str,
    todo_id: &str,
    title: &str,
    ok: bool,
) -> Result<(), String> {
    let mut progress = base_progress(session_home, session_id);
    set_todo_status(
        &mut progress.todos,
        todo_id,
        if ok { "done" } else { "skipped" },
    );
    progress.phase = String::from("executing_todo");
    progress.current_todo_id = Some(todo_id.to_string());
    let total = progress.todos.len();
    let done = done_count(&progress.todos);
    progress.current_task_desc = if ok {
        format!("已完成：{title}（{done}/{total}）")
    } else {
        format!("跳过：{title}（{done}/{total}）")
    };
    write_progress(session_home, progress)
}

pub fn publish_writer_started(session_home: &Path, session_id: &str) -> Result<(), String> {
    let mut progress = base_progress(session_home, session_id);
    progress.phase = String::from("executing_todo");
    progress.current_todo_id = None;
    progress.current_task_desc = String::from("正在撰写分析报告…");
    write_progress(session_home, progress)
}

pub fn publish_done(session_home: &Path, session_id: &str) -> Result<(), String> {
    let mut progress = base_progress(session_home, session_id);
    let total = progress.todos.len();
    for todo in &mut progress.todos {
        if todo.status == "pending" || todo.status == "in_progress" {
            todo.status = String::from("done");
        }
    }
    progress.phase = String::from("done");
    progress.current_todo_id = None;
    progress.current_task_desc = if total > 0 {
        format!("分析完成（{total}/{total}）")
    } else {
        String::from("分析完成")
    };
    // Keep plan_title + todos in final snapshot for task API / admin UI.
    write_progress(session_home, progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multi_agent::plan::{AnalysisPlan, AnalysisPlanTodo};
    use std::fs;

    fn temp_session() -> (tempfile::TempDir, std::path::PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("sess");
        fs::create_dir_all(&home).unwrap();
        (dir, home, String::from("sess-1"))
    }

    fn sample_plan() -> AnalysisPlan {
        AnalysisPlan {
            plan_title: String::from("测试分析"),
            todos: vec![
                AnalysisPlanTodo {
                    id: String::from("1"),
                    title: String::from("营收"),
                    question: String::from("q1"),
                },
                AnalysisPlanTodo {
                    id: String::from("2"),
                    title: String::from("订单"),
                    question: String::from("q2"),
                },
            ],
        }
    }

    #[test]
    fn progress_advances_through_query_and_done() {
        let (_dir, home, sid) = temp_session();
        publish_plan(&home, &sid, &sample_plan()).unwrap();
        on_query_started(&home, &sid, "1", "营收").unwrap();
        on_query_finished(&home, &sid, "1", "营收", true).unwrap();
        on_query_finished(&home, &sid, "2", "订单", true).unwrap();
        publish_done(&home, &sid).unwrap();
        let p = read_task_progress(&home).unwrap();
        assert_eq!(p.phase, "done");
        assert!(p.todos.iter().all(|t| t.status == "done"));
    }
}
