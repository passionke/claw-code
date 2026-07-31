//! Gateway `Agent` tool → orchestration timeline events. Author: kejiqing

use std::sync::Arc;

use tools::{run_agent_job_to_terminal, spawn_agent_job, AgentJob, AgentOutput};

use crate::multi_agent::{now_ms, EventBus};

fn attach_agent_terminal_events(bus: &EventBus, job: AgentJob) -> Result<AgentJob, String> {
    let agent_id = job.manifest.agent_id.clone();
    let title = job.manifest.description.clone();
    let start_ms = now_ms();
    bus.agent_started(&agent_id, &title)?;
    let bus_done = bus.clone();
    let agent_id_done = agent_id.clone();
    let hook: tools::AgentTerminalHook = Arc::new(move |status, err| {
        let duration_ms = now_ms().saturating_sub(start_ms);
        if status == "completed" {
            let _ = bus_done.agent_done(&agent_id_done, duration_ms);
        } else {
            let _ =
                bus_done.agent_failed(&agent_id_done, err.as_deref().unwrap_or("sub-agent failed"));
        }
    });
    Ok(job.with_terminal_hook(hook))
}

/// Spawn a Background sub-agent and append `agent_started` / `agent_done|failed`. Author: kejiqing
pub fn spawn_gateway_agent_with_events(bus: &EventBus, job: AgentJob) -> Result<(), String> {
    let agent_id = job.manifest.agent_id.clone();
    let job = attach_agent_terminal_events(bus, job).inspect_err(|e| {
        let _ = bus.agent_failed(&agent_id, e);
    })?;
    spawn_agent_job(job).inspect_err(|e| {
        let _ = bus.agent_failed(&agent_id, e);
    })
}

/// Run a Foreground sub-agent to terminal with the same orchestration events. Author: kejiqing
pub fn run_gateway_agent_foreground(bus: &EventBus, job: AgentJob) -> Result<AgentOutput, String> {
    let agent_id = job.manifest.agent_id.clone();
    let job = attach_agent_terminal_events(bus, job).inspect_err(|e| {
        let _ = bus.agent_failed(&agent_id, e);
    })?;
    // terminal_hook emits done/failed; do not double-emit on Err. Author: kejiqing
    run_agent_job_to_terminal(&job)
}
