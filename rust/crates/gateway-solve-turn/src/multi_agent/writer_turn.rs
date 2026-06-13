//! Phase3 report writer LLM turn. Author: kejiqing

use std::path::Path;

use serde_json::json;

use crate::multi_agent::phase_turn::{run_phase_turn, writer_allowed_tools};
use crate::multi_agent::phases::{load_phase_prompt, DEFAULT_WRITER_MD};
use crate::multi_agent::plan::AnalysisPlan;
use crate::multi_agent::query_fanout::QueryResult;
use crate::multi_agent::timings::{now_ms, MultiAgentTimings};
use crate::project_orchestration::SolveOrchestrationConfig;
use crate::{DirectApiClient, DirectToolExecutor, GatewaySolveTurnError};

pub fn run_writer_turn(
    work_dir: &Path,
    user_prompt: &str,
    plan: &AnalysisPlan,
    query_results: &[QueryResult],
    base_allowed: &[String],
    orch: &SolveOrchestrationConfig,
    model: &str,
    _mcp_tools: Vec<api::ToolDefinition>,
    session_id: &str,
    executor: &DirectToolExecutor,
    timings: &mut MultiAgentTimings,
) -> Result<String, GatewaySolveTurnError> {
    let started = now_ms();
    let system = load_phase_prompt(work_dir, "writer.md", DEFAULT_WRITER_MD);
    let allowed = writer_allowed_tools(base_allowed);
    let api = DirectApiClient::new(
        model.to_string(),
        &allowed,
        vec![], // writer: no MCP
        session_id.to_string(),
    )
    .map_err(|e| GatewaySolveTurnError {
        status: 500,
        message: e.message,
    })?;
    let phase_executor = executor.clone_with_allowed_tools(allowed);
    let summaries: Vec<_> = query_results
        .iter()
        .map(|r| {
            json!({
                "todoId": r.todo_id,
                "ok": r.ok,
                "summary": r.summary,
            })
        })
        .collect();
    let user = format!(
        "Original user request:\n{user_prompt}\n\nAnalysis plan:\n{}\n\nSub-query summaries (JSON):\n{}\n\nWrite the final markdown report.",
        serde_json::to_string_pretty(plan).unwrap_or_default(),
        serde_json::to_string_pretty(&summaries).unwrap_or_default()
    );
    let (text, _iters) = run_phase_turn(
        user,
        api,
        phase_executor,
        vec![system],
        orch.writer_max_iter,
        true,
        executor.turn_timing(),
    )?;
    timings.push("writer", started, now_ms(), None);
    Ok(text)
}
