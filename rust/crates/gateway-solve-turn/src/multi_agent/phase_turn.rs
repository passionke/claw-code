//! Short-lived ConversationRuntime for a single multi-agent phase. Author: kejiqing
use std::sync::Arc;

use runtime::{ConversationRuntime, PermissionMode, PermissionPolicy, Session};

use crate::{
    assistant_report_text_from_turn, DirectApiClient, DirectToolExecutor, GatewaySolveTurnError,
    SolveTimingRecorder,
};

fn err(status: u16, msg: impl Into<String>) -> GatewaySolveTurnError {
    GatewaySolveTurnError {
        status,
        message: msg.into(),
    }
}

const HTTP_INTERNAL: u16 = 500;

/// Run one LLM phase turn on an ephemeral session; returns assistant text.
pub fn run_phase_turn(
    user_text: String,
    api_client: DirectApiClient,
    tool_executor: DirectToolExecutor,
    system_prompt: Vec<String>,
    max_iterations: usize,
    stream_text_to_report: bool,
    turn_timing: Option<Arc<SolveTimingRecorder>>,
) -> Result<(String, usize), GatewaySolveTurnError> {
    let api_client = api_client.with_stream_report_deltas(stream_text_to_report);
    let mut session = Session::new();
    session
        .push_user_text(user_text)
        .map_err(|e| err(HTTP_INTERNAL, format!("push user message failed: {e}")))?;

    let policy = PermissionPolicy::new(PermissionMode::DangerFullAccess);
    let mut runtime =
        ConversationRuntime::new(session, api_client, tool_executor, policy, system_prompt);
    runtime = runtime.with_max_iterations(max_iterations);
    if let Some(timing) = turn_timing {
        runtime = runtime.with_turn_timing(timing);
    }

    let result = runtime
        .run_turn_after_user_message(None)
        .map_err(|e| err(HTTP_INTERNAL, format!("phase runtime failed: {e}")))?;

    let message = assistant_report_text_from_turn(&result.assistant_messages);

    Ok((message, result.iterations))
}

/// Allowed tool names for planner phase (StructuredOutput always included — kernel invariant).
#[must_use]
pub fn planner_allowed_tools(base: &[String]) -> Vec<String> {
    let mut out = vec![String::from("StructuredOutput")];
    for name in filter_allowed(base, &["read_file"]) {
        if !out.iter().any(|n| n == &name) {
            out.push(name);
        }
    }
    out
}

#[must_use]
pub fn writer_allowed_tools(base: &[String]) -> Vec<String> {
    filter_allowed(base, &["read_file", "Skill"])
}

fn filter_allowed(base: &[String], want: &[&str]) -> Vec<String> {
    if base.is_empty() {
        return want.iter().map(|s| (*s).to_string()).collect();
    }
    want.iter()
        .filter(|w| {
            base.iter().any(|b| {
                b == *w
                    || b.strip_suffix('*')
                        .is_some_and(|prefix| w.starts_with(prefix))
            })
        })
        .map(|s| (*s).to_string())
        .collect()
}

/// Serialize events batch for narrator user message.
pub fn format_events_for_narrator(
    events: &[crate::multi_agent::event_bus::OrchestrationEvent],
) -> String {
    use serde_json::json;
    let lines: Vec<_> = events
        .iter()
        .map(|e| {
            json!({
                "kind": e.kind,
                "todoId": e.todo_id,
                "message": e.message,
                "durationMs": e.duration_ms,
                "error": e.error,
                "hasPlan": e.plan.is_some(),
            })
            .to_string()
        })
        .collect();
    format!(
        "New orchestration events since last update:\n{}\n\nCall report_progress once with user-visible status.",
        lines.join("\n")
    )
}
