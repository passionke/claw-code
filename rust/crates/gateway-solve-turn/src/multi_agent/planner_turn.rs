//! Phase1 planner LLM turn. Author: kejiqing

use std::path::Path;

use crate::multi_agent::phase_turn::{planner_allowed_tools, run_phase_turn};
use crate::multi_agent::phases::{load_phase_prompt, DEFAULT_PLANNER_MD};
use crate::multi_agent::plan::{parse_plan_from_text, AnalysisPlan};
use crate::multi_agent::timings::{now_ms, MultiAgentTimings};
use crate::project_orchestration::SolveOrchestrationConfig;
use crate::{DirectApiClient, DirectToolExecutor, GatewaySolveTurnError};

pub fn run_planner_turn(
    work_dir: &Path,
    user_prompt: &str,
    schema_section: Option<String>,
    base_allowed: &[String],
    orch: &SolveOrchestrationConfig,
    model: &str,
    mcp_tools: Vec<api::ToolDefinition>,
    session_id: &str,
    executor: &DirectToolExecutor,
    timings: &mut MultiAgentTimings,
) -> Result<AnalysisPlan, GatewaySolveTurnError> {
    let started = now_ms();
    let system = load_phase_prompt(work_dir, "planner.md", DEFAULT_PLANNER_MD);
    let mut system_prompt = vec![system];
    if let Some(schema) = schema_section {
        system_prompt.push(schema);
    }
    let allowed = planner_allowed_tools(base_allowed);
    let api = DirectApiClient::new(
        model.to_string(),
        &allowed,
        if allowed.is_empty() {
            vec![]
        } else {
            mcp_tools
                .into_iter()
                .filter(|t| allowed.iter().any(|a| a == &t.name))
                .collect()
        },
        session_id.to_string(),
    )
    .map_err(|e| crate::GatewaySolveTurnError {
        status: 500,
        message: e.message,
    })?;
    let phase_executor = executor.clone_with_allowed_tools(allowed);
    let user = format!(
        "User analysis request:\n{user_prompt}\n\nRespond with AnalysisPlan JSON only."
    );
    let (text, _iters) = run_phase_turn(
        user,
        api,
        phase_executor,
        system_prompt,
        orch.planner_max_iter,
        false,
    )?;
    let plan = parse_plan_from_text(&text).map_err(|e| crate::GatewaySolveTurnError {
        status: 500,
        message: format!("planner output invalid: {e}; raw={text}"),
    })?;
    timings.push("planner", started, now_ms(), None);
    Ok(plan)
}
