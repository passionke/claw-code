//! OTEL spans for `run_gateway_solve_turn` (independent of JSONL `SessionTracer`). Author: kejiqing

use telemetry::{context_from_env_traceparent, log_prompts_enabled, otel_enabled, OtelSpanGuard};

use crate::GatewayMcpCallContext;

pub struct SolveTurnOtelGuard {
    inner: Option<OtelSpanGuard>,
    outcome: Outcome,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Pending,
    Ok,
    Error,
}

impl SolveTurnOtelGuard {
    #[must_use]
    pub fn start(mcp: &GatewayMcpCallContext, user_prompt: &str) -> Self {
        if !otel_enabled() {
            return Self {
                inner: None,
                outcome: Outcome::Pending,
            };
        }
        let turn_id = std::env::var("CLAW_TURN_ID")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| mcp.turn_id.clone());
        let parent = context_from_env_traceparent();
        let guard = OtelSpanGuard::start("gateway-solve-turn", "gateway_solve_turn", Some(&parent));
        if let Some(ref g) = guard {
            g.set_langfuse_trace_attrs(mcp.clawcode_session_id(), &turn_id, &mcp.request_id);
            g.set_attribute("langfuse.trace.name", "gateway_solve_turn");
            if log_prompts_enabled() {
                let preview: String = user_prompt.chars().take(8000).collect();
                g.set_attribute("langfuse.trace.input", preview);
            }
        }
        Self {
            inner: guard,
            outcome: Outcome::Pending,
        }
    }

    pub fn enter(&self) -> Option<telemetry::OtelContextGuard> {
        self.inner.as_ref().map(|g| g.enter())
    }

    pub fn mark_ok(&mut self, output_text: &str) {
        if let Some(ref g) = self.inner {
            if log_prompts_enabled() {
                let preview: String = output_text.chars().take(8000).collect();
                g.set_attribute("langfuse.trace.output", preview);
            }
            g.set_ok();
        }
        self.outcome = Outcome::Ok;
    }

    pub fn mark_error(&mut self, message: &str) {
        if let Some(ref g) = self.inner {
            g.set_error(message);
        }
        self.outcome = Outcome::Error;
    }
}

impl Drop for SolveTurnOtelGuard {
    fn drop(&mut self) {
        if self.outcome == Outcome::Pending {
            if let Some(ref g) = self.inner {
                g.set_ok();
            }
        }
    }
}
