//! Gateway `Agent` tool → orchestration timeline events. Author: kejiqing

use std::sync::Arc;

use tools::AgentJob;

use crate::multi_agent::{now_ms, EventBus};

/// Spawn a sub-agent and append `agent_started` / `agent_done|failed` to orchestration-events. Author: kejiqing
pub fn spawn_gateway_agent_with_events(bus: &EventBus, job: AgentJob) -> Result<(), String> {
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
    let job = job.with_terminal_hook(hook);
    tools::spawn_agent_job(job).inspect_err(|e| {
        let _ = bus.agent_failed(&agent_id, e);
    })
}
