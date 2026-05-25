//! Parallel MCP fan-out (no LLM). Concurrency: `CLAW_MCP_MAX_CONCURRENT` only. Author: kejiqing

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use futures_util::future::join_all;
use serde_json::{json, Value};

use crate::multi_agent::event_bus::EventBus;
use crate::multi_agent::plan::{AnalysisPlan, AnalysisPlanTodo};
use crate::multi_agent::progress_sync::{on_query_finished, on_query_started};
use crate::multi_agent::timings::{now_ms, MultiAgentTimings};
use crate::project_orchestration::SolveOrchestrationConfig;
use crate::sqlbot_preflight::{sqlbot_mcp_payload_is_error, SqlbotQueryContext};
use crate::DirectToolExecutor;
use runtime::ToolError;

pub const ANALYSIS_RESULTS_DIR: &str = ".claw/analysis-results";

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub todo_id: String,
    pub ok: bool,
    pub summary: String,
    pub raw_truncated: String,
}

fn compress_mcp_result(raw: &str, max_chars: usize) -> (String, String) {
    let trimmed = raw.trim();
    let payload = sqlbot_inner_text(trimmed).unwrap_or_else(|| trimmed.to_string());
    if let Ok(v) = serde_json::from_str::<Value>(&payload) {
        let summary = v
            .get("report_message")
            .or_else(|| v.get("message"))
            .or_else(|| v.get("msg"))
            .or_else(|| v.get("analysis"))
            .or_else(|| v.pointer("/data/report_message"))
            .or_else(|| v.pointer("/data/message"))
            .or_else(|| v.pointer("/data/analysis"))
            .and_then(Value::as_str)
            .unwrap_or(&payload);
        let summary = if summary.chars().count() > max_chars {
            format!("{}...", summary.chars().take(max_chars).collect::<String>())
        } else {
            summary.to_string()
        };
        let raw_trunc = if trimmed.chars().count() > max_chars * 2 {
            format!(
                "{}...",
                trimmed.chars().take(max_chars * 2).collect::<String>()
            )
        } else {
            trimmed.to_string()
        };
        return (summary, raw_trunc);
    }
    let summary = if payload.chars().count() > max_chars {
        format!("{}...", payload.chars().take(max_chars).collect::<String>())
    } else {
        payload.clone()
    };
    (summary.clone(), summary)
}

fn sqlbot_inner_text(raw: &str) -> Option<String> {
    let outer: Value = serde_json::from_str(raw).ok()?;
    outer
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn build_fanout_tool_args(
    sqlbot_ctx: &SqlbotQueryContext,
    question: &str,
    isolated: bool,
) -> Value {
    if isolated {
        let mut args = json!({
            "token": sqlbot_ctx.token,
            "question": question,
            "stream": false,
        });
        if let Some(ds) = sqlbot_ctx.datasource_id {
            args["datasource_id"] = json!(ds);
        }
        args
    } else {
        json!({
            "token": sqlbot_ctx.token,
            "chat_id": sqlbot_ctx.chat_id,
            "question": question,
        })
    }
}

fn write_result_file(
    session_home: &Path,
    todo_id: &str,
    result: &QueryResult,
) -> Result<(), String> {
    let dir = session_home.join(ANALYSIS_RESULTS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create results dir: {e}"))?;
    let path = dir.join(format!("{todo_id}.json"));
    let body = json!({
        "todoId": result.todo_id,
        "ok": result.ok,
        "summary": result.summary,
        "rawTruncated": result.raw_truncated,
    });
    std::fs::write(path, serde_json::to_vec_pretty(&body).unwrap_or_default())
        .map_err(|e| format!("write result file: {e}"))?;
    Ok(())
}

async fn run_one_query(
    session_home: &Path,
    session_id: &str,
    executor: Arc<DirectToolExecutor>,
    tool_name: String,
    isolated_fanout: bool,
    sqlbot_ctx: SqlbotQueryContext,
    todo: AnalysisPlanTodo,
    event_bus: EventBus,
) -> QueryResult {
    let _ = event_bus.query_started(&todo.id, &todo.title);
    let _ = on_query_started(session_home, session_id, &todo.id, &todo.title);
    let started = Instant::now();
    let args = build_fanout_tool_args(&sqlbot_ctx, &todo.question, isolated_fanout);
    let input = args.to_string();
    let ex = Arc::clone(&executor);
    let tn = tool_name.clone();
    let out = tokio::task::spawn_blocking(move || ex.call_tool(&tn, &input))
        .await
        .map_err(|e| ToolError::new(format!("query join: {e}")));
    let out = match out {
        Ok(r) => r,
        Err(e) => {
            let message = e.to_string();
            let _ = event_bus.query_failed(&todo.id, &message);
            let _ = on_query_finished(session_home, session_id, &todo.id, &todo.title, false);
            return QueryResult {
                todo_id: todo.id.clone(),
                ok: false,
                summary: message.clone(),
                raw_truncated: message,
            };
        }
    };
    let elapsed = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
    match out {
        Ok(raw) => {
            let payload_error = sqlbot_mcp_payload_is_error(&raw);
            let (summary, raw_truncated) = compress_mcp_result(&raw, 2048);
            if payload_error {
                let _ = event_bus.query_failed(&todo.id, &summary);
            } else {
                let _ = event_bus.query_done(&todo.id, elapsed);
            }
            let _ = on_query_finished(
                session_home,
                session_id,
                &todo.id,
                &todo.title,
                !payload_error,
            );
            QueryResult {
                todo_id: todo.id,
                ok: !payload_error,
                summary,
                raw_truncated,
            }
        }
        Err(e) => {
            let message = e.to_string();
            let _ = event_bus.query_failed(&todo.id, &message);
            let _ = on_query_finished(session_home, session_id, &todo.id, &todo.title, false);
            QueryResult {
                todo_id: todo.id,
                ok: false,
                summary: message.clone(),
                raw_truncated: message,
            }
        }
    }
}

/// Fan out plan todos to MCP analysis tools (`CLAW_MCP_MAX_CONCURRENT` limits in-flight calls).
pub async fn run_query_fanout(
    session_home: &Path,
    session_id: &str,
    executor: Arc<DirectToolExecutor>,
    tool_names: &HashSet<String>,
    orch: &SolveOrchestrationConfig,
    sqlbot_ctx: &SqlbotQueryContext,
    plan: &AnalysisPlan,
    event_bus: &EventBus,
    timings: &mut MultiAgentTimings,
) -> Result<Vec<QueryResult>, String> {
    let tool_name = executor
        .resolve_query_fanout_tool(tool_names, orch.query_mcp_tool.as_deref())
        .ok_or_else(|| {
            format!(
                "no MCP tool eligible for parallel query_fanout. \
                 Use a tool whose description contains `parallel-friendly` (e.g. SQLBot mcp_isolated_question_analysis), \
                 set solveOrchestrationJson.queryMcpTool, or add mcpServers.<server>.toolAnnotations.<rawTool>.readOnlyHint=true. \
                 registered_mcp_tools={:?}",
                tool_names.iter().collect::<Vec<_>>()
            )
        })?;
    let isolated_fanout = executor.query_fanout_uses_isolated_args(&tool_name);
    let started = now_ms();
    let bus = event_bus.clone();
    let ctx = sqlbot_ctx.clone();
    let home = session_home.to_path_buf();
    let sid = session_id.to_string();
    let futs: Vec<_> = plan
        .todos
        .iter()
        .cloned()
        .map(|todo| {
            let ex = Arc::clone(&executor);
            let tn = tool_name.clone();
            let iso = isolated_fanout;
            let eb = bus.clone();
            let c = ctx.clone();
            let h = home.clone();
            let sid = sid.clone();
            async move { run_one_query(&h, &sid, ex, tn, iso, c, todo, eb).await }
        })
        .collect();
    let results = join_all(futs).await;
    for r in &results {
        write_result_file(session_home, &r.todo_id, r)?;
    }
    let ended = now_ms();
    timings.push(
        "query_fanout",
        started,
        ended,
        Some(format!("n={}", results.len())),
    );
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_extracts_report_message() {
        let raw = r#"{"report_message":"sales up 10%"}"#;
        let (s, _) = compress_mcp_result(raw, 100);
        assert_eq!(s, "sales up 10%");
    }

    #[test]
    fn isolated_fanout_args_skip_chat_id() {
        let ctx = SqlbotQueryContext {
            token: "t".into(),
            chat_id: 99,
            datasource_id: Some(34),
        };
        let args = build_fanout_tool_args(&ctx, "q?", true);
        assert_eq!(args["token"], "t");
        assert_eq!(args["question"], "q?");
        assert_eq!(args["stream"], false);
        assert_eq!(args["datasource_id"], 34);
        assert!(args.get("chat_id").is_none());
    }

    #[test]
    fn compress_extracts_inner_sqlbot_payload() {
        let inner = r#"{"code":0,"data":{"report_message":"revenue flat"}}"#;
        let raw = format!(r#"{{"content":[{{"type":"text","text":{inner:?}}}]}}"#);
        let (s, _) = compress_mcp_result(&raw, 100);
        assert_eq!(s, "revenue flat");
    }
}
