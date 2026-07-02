//! Multi-agent gateway solve orchestrator. Author: kejiqing

use std::path::Path;
use std::sync::Arc;

use runtime::{
    apply_config_env_if_unset, gateway_schema_prompt_section, ConfigLoader, RuntimeConfig, Session,
};
use serde_json::json;
use tracing::info;

use crate::multi_agent::event_bus::EventBus;
use crate::multi_agent::narrator_lane::{spawn_narrator_lane, NarratorHandle};
use crate::multi_agent::planner_turn::run_planner_turn;
use crate::multi_agent::progress_sync::{publish_done, publish_plan, publish_writer_started};
use crate::multi_agent::query_fanout::run_query_fanout;
use crate::multi_agent::timings::MultiAgentTimings;
use crate::multi_agent::writer_turn::run_writer_turn;
use crate::project_orchestration::SolveOrchestrationConfig;
use crate::project_preflight;
use crate::solve_timing::append_solve_timing_point;
use crate::sqlbot_preflight::sqlbot_query_context_from_session;
use crate::{
    default_system_date, err, gateway_solve_session_persistence_path, initialize_mcp_runtime,
    reset_task_progress, truncate_progress_history, DirectToolExecutor, GatewayMcpCallContext,
    GatewaySolveTurnError, SolveTimingRecorder, HTTP_INTERNAL,
};

#[allow(clippy::too_many_arguments)]
pub fn run_multi_agent_solve_turn(
    work_dir: &Path,
    work_root: &Path,
    prompt: &str,
    model: Option<&str>,
    _timeout_seconds: u64,
    mcp: GatewayMcpCallContext,
    allowed_tools: Vec<String>,
    _max_iterations: usize,
    orch: SolveOrchestrationConfig,
) -> Result<(i32, String, Option<serde_json::Value>), GatewaySolveTurnError> {
    let clawcode_session_id = mcp.clawcode_session_id();
    info!(
        target: "claw_gateway_orchestration",
        orchestration = "multi_agent",
        phase = "start",
        session_id = %clawcode_session_id,
        turn_id = %mcp.turn_id,
        request_id = %mcp.request_id,
        trace_id = %mcp.trace_id,
        "multi_agent solve turn starting"
    );

    let project_cfg = match crate::project_config_loader_root() {
        Some(root) => ConfigLoader::default_for(&root).load().map_err(|e| {
            err(
                HTTP_INTERNAL,
                format!("load claw config from {}: {e}", root.display()),
            )
        })?,
        None => RuntimeConfig::empty(),
    };
    apply_config_env_if_unset(&project_cfg);
    let turn_id_attr = std::env::var("CLAW_TURN_ID").ok();
    let _ = append_solve_timing_point(
        work_dir,
        "bootstrap_project_config_loaded",
        turn_id_attr.as_deref(),
    );
    let effective_model = model
        .map(str::to_string)
        .or_else(|| std::env::var("CLAW_DEFAULT_MODEL").ok())
        .or_else(|| project_cfg.model().map(str::to_string))
        .unwrap_or_else(|| "openai/deepseek-v4-pro".to_string());
    let narrator_model = orch
        .narrator_model
        .clone()
        .unwrap_or_else(|| effective_model.clone());

    let (
        runtime_mcp_tools,
        runtime_mcp_tool_names,
        concurrent_mcp_tool_names,
        parallel_friendly_mcp_tool_names,
        runtime_mcp_manager,
    ) = initialize_mcp_runtime(work_dir)?;
    let _ = append_solve_timing_point(work_dir, "bootstrap_mcp_ready", turn_id_attr.as_deref());

    reset_task_progress(work_dir, clawcode_session_id)
        .map_err(|e| err(HTTP_INTERNAL, format!("reset task progress failed: {e}")))?;
    let _ = truncate_progress_history(work_dir);
    let turn_timing = Arc::new(SolveTimingRecorder::new(work_dir));

    let gateway_jsonl = gateway_solve_session_persistence_path(work_dir);
    let session_is_continuation = gateway_jsonl.exists();
    let mut session = if session_is_continuation {
        Session::load_from_path(&gateway_jsonl).map_err(|e| {
            err(
                HTTP_INTERNAL,
                format!("load gateway session transcript: {e}"),
            )
        })?
    } else {
        Session::new().with_persistence_path(gateway_jsonl.clone())
    }
    .with_workspace_root(work_dir);

    let session_tracer = crate::gateway_session_tracer(&mcp.request_id, work_root);
    let async_runtime = tokio::runtime::Handle::try_current().map_err(|_| {
        err(
            HTTP_INTERNAL,
            "gateway solve requires a Tokio runtime (gateway-solve-once must call run_gateway_solve_turn inside rt.enter())",
        )
    })?;

    let mut tool_executor = DirectToolExecutor::new(
        work_dir.to_path_buf(),
        mcp.clone(),
        effective_model.clone(),
        allowed_tools.clone(),
        runtime_mcp_manager.clone(),
        runtime_mcp_tool_names.clone(),
        concurrent_mcp_tool_names,
        parallel_friendly_mcp_tool_names,
        session_tracer,
        Some(Arc::clone(&turn_timing)),
        async_runtime,
    );

    session
        .push_user_text(prompt)
        .map_err(|e| err(HTTP_INTERNAL, format!("push user message failed: {e}")))?;

    let event_bus = EventBus::new(work_dir);
    let _ = event_bus.session_started();
    let mut timings = MultiAgentTimings::load(work_dir);

    let narrator: NarratorHandle = spawn_narrator_lane(
        work_dir.to_path_buf(),
        clawcode_session_id.to_string(),
        orch.clone(),
        narrator_model,
        tool_executor.clone(),
        event_bus.clone(),
    )?;

    let language_pipeline_json =
        crate::project_language_pipeline::load_language_pipeline_json(work_dir);
    let turn_id_for_preflight = turn_id_attr
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("unknown");
    let mut system_prompt = Vec::<String>::new();
    let preflight_params = crate::preflight_runner::PreflightRunParams {
        session_home: work_dir,
        session: &mut session,
        system_prompt: &mut system_prompt,
        executor: &mut tool_executor,
        is_continuation: session_is_continuation,
        user_prompt: prompt,
        turn_id: turn_id_for_preflight,
        session_id: clawcode_session_id,
        model: &effective_model,
        extra_session: mcp
            .extra_session
            .clone()
            .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null)),
    };
    let preflight_report =
        project_preflight::run_solve_preflight(preflight_params, &language_pipeline_json)?;
    if preflight_report.ran_session_first_turn {
        let _ = event_bus.preflight_done();
    }

    let executor = Arc::new(tool_executor);
    let schema_section = if preflight_report.ran_session_first_turn {
        gateway_schema_prompt_section(work_dir)
    } else {
        None
    };

    let plan = run_planner_turn(
        work_dir,
        prompt,
        schema_section,
        &allowed_tools,
        &orch,
        &effective_model,
        runtime_mcp_tools,
        clawcode_session_id,
        executor.as_ref(),
        &mut timings,
    )?;
    let _ = event_bus.plan_ready(&plan);
    let _ = publish_plan(work_dir, clawcode_session_id, &plan);

    let sqlbot_ctx = sqlbot_query_context_from_session(&session).ok_or_else(|| {
        err(
            HTTP_INTERNAL,
            "multi_agent query_fanout: missing SQLBot token/chat_id — enable sqlbot_mcp_start preflight or continue a session with mcp_start in transcript",
        )
    })?;

    let query_results = tokio::runtime::Handle::current()
        .block_on(run_query_fanout(
            work_dir,
            clawcode_session_id,
            Arc::clone(&executor),
            &runtime_mcp_tool_names,
            &orch,
            &sqlbot_ctx,
            &plan,
            &event_bus,
            &mut timings,
        ))
        .map_err(|e| err(HTTP_INTERNAL, e))?;

    let _ = event_bus.writer_started();
    let _ = publish_writer_started(work_dir, clawcode_session_id);
    let report = run_writer_turn(
        work_dir,
        prompt,
        &plan,
        &query_results,
        &allowed_tools,
        &orch,
        &effective_model,
        vec![],
        clawcode_session_id,
        executor.as_ref(),
        &mut timings,
    )?;
    let _ = event_bus.writer_done();
    let _ = publish_done(work_dir, clawcode_session_id);

    narrator.stop_and_join();
    let _ = timings.save(work_dir);

    let _default_date = default_system_date();

    let out_json = json!({
        "model": effective_model,
        "orchestration": "multi_agent_analysis",
        "iterations": 0,
        "message": report,
        "planTitle": plan.plan_title,
        "todoCount": plan.todos.len(),
        "usage": {
            "input_tokens": 0,
            "output_tokens": 0,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0
        }
    });
    Ok((
        0,
        serde_json::to_string(&out_json).unwrap_or_default(),
        Some(out_json),
    ))
}
