//! Run project preflight steps for worker lifecycle events (pre-Landlock).
//! Author: kejiqing

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle};
use preflight_spi::{
    build_worker_init_context, filter_step_indices_for_event, normalize_pipeline_steps,
    parse_pipeline_value, validate_context_for_event, validate_effects_for_event,
    LifecycleEventContext, PreflightFilterContext, PreflightImpl, PreflightLifecycleEvent,
    PreflightOutcome, PreflightResponseStatus, PreflightSpiResponse, PreflightStep, SPI_VERSION,
};
use serde_json::{json, Value};
use tracing::{info, warn};

/// Load normalized steps from project `solve_preflight_json`.
#[must_use]
pub fn steps_from_solve_preflight_json(value: &Value) -> Vec<PreflightStep> {
    parse_pipeline_value(value)
        .map(|cfg| normalize_pipeline_steps(&cfg))
        .unwrap_or_default()
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Build guest shell that feeds SPI JSON to a subprocess plugin and checks status. Author: kejiqing
fn guest_spi_script(command: &[String], request_json: &str) -> String {
    let prog = command
        .iter()
        .map(|c| shell_quote(c))
        .collect::<Vec<_>>()
        .join(" ");
    // Write request to a temp file to avoid shell escaping issues with large JSON.
    format!(
        r#"set -euo pipefail
REQ=$(mktemp)
trap 'rm -f "$REQ"' EXIT
cat >"$REQ" <<'CLAW_PREFLIGHT_SPI_EOF'
{request_json}
CLAW_PREFLIGHT_SPI_EOF
OUT=$(mktemp)
trap 'rm -f "$REQ" "$OUT"' EXIT
set +e
{prog} <"$REQ" >"$OUT"
EC=$?
set -e
if [ "$EC" -ne 0 ]; then
  echo "preflight plugin exit $EC" >&2
  cat "$OUT" >&2 || true
  exit "$EC"
fi
python3 - <<'PY' "$OUT"
import json, sys
path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    data = json.load(f)
status = data.get("status", "")
if status == "error":
    raise SystemExit(data.get("message") or "preflight error")
if status not in ("ok", "skip"):
    raise SystemExit(f"unexpected preflight status: {{status!r}}")
print(status)
PY
"#
    )
}

fn spi_request_value(
    event: PreflightLifecycleEvent,
    step: &PreflightStep,
    ctx: &LifecycleEventContext,
) -> Result<Value, String> {
    validate_context_for_event(event, ctx)?;
    Ok(json!({
        "spiVersion": SPI_VERSION,
        "event": event.as_str(),
        "step": step,
        "context": ctx,
        "artifacts": [],
    }))
}

/// Run matching subprocess steps for a worker lifecycle event inside the guest.
pub async fn run_worker_lifecycle_event_on_guest(
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
    steps: &[PreflightStep],
    event: PreflightLifecycleEvent,
    ctx: &LifecycleEventContext,
) -> Result<usize, String> {
    if !event.is_pre_jail() {
        return Err(format!(
            "run_worker_lifecycle_event_on_guest only supports pre-jail events (got {})",
            event.as_str()
        ));
    }
    validate_context_for_event(event, ctx)?;
    let filter = PreflightFilterContext {
        is_continuation: false,
        session_first_turn_satisfied: false,
    };
    let indices = filter_step_indices_for_event(steps, event, filter);
    let mut ran = 0usize;
    for idx in indices {
        let step = &steps[idx];
        let impl_kind = step.r#impl.clone().unwrap_or(PreflightImpl::Subprocess {
            command: vec![],
        });
        match impl_kind {
            PreflightImpl::Builtin { handler } => {
                warn!(
                    target: "claw_worker_lifecycle_preflight",
                    plugin_id = %step.plugin_id,
                    %handler,
                    event = %event.as_str(),
                    "skipping builtin preflight on worker lifecycle (subprocess only)"
                );
            }
            PreflightImpl::Subprocess { command } => {
                if command.is_empty() {
                    return Err(format!(
                        "preflight step {} on {} has empty subprocess command",
                        step.plugin_id,
                        event.as_str()
                    ));
                }
                let req = spi_request_value(event, step, ctx)?;
                let req_json = serde_json::to_string(&req)
                    .map_err(|e| format!("encode lifecycle SPI request: {e}"))?;
                let script = guest_spi_script(&command, &req_json);
                let stdout = client
                    .exec_shell_script_stdout(handle, &script, None)
                    .await
                    .map_err(|e| {
                        format!(
                            "worker lifecycle {} plugin {}: {e}",
                            event.as_str(),
                            step.plugin_id
                        )
                    })?;
                // Best-effort parse last line as status; empty ok if python printed ok/skip.
                if let Ok(resp) = serde_json::from_str::<PreflightSpiResponse>(stdout.trim()) {
                    if resp.status == PreflightResponseStatus::Error {
                        return Err(resp.message.unwrap_or_else(|| {
                            format!("preflight {} returned error", step.plugin_id)
                        }));
                    }
                    validate_effects_for_event(event, &resp.effects)?;
                }
                ran += 1;
                info!(
                    target: "claw_worker_lifecycle_preflight",
                    plugin_id = %step.plugin_id,
                    event = %event.as_str(),
                    "worker lifecycle preflight step ok"
                );
            }
        }
    }
    Ok(ran)
}

/// After sandbox create: `worker.init.start` then `worker.init.end`. Author: kejiqing
pub async fn run_worker_init_on_create(
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
    solve_preflight_json: &Value,
    proj_id: i64,
    worker_id: &str,
    template_id: &str,
    worker_profile_mode: &str,
) -> Result<(), String> {
    let steps = steps_from_solve_preflight_json(solve_preflight_json);
    let started = std::time::Instant::now();
    let mut ctx = build_worker_init_context(
        proj_id,
        worker_id,
        "/claw_ds",
        template_id,
        &handle.sandbox_id,
        worker_profile_mode,
    );
    let n_start = run_worker_lifecycle_event_on_guest(
        client,
        handle,
        &steps,
        PreflightLifecycleEvent::WorkerInitStart,
        &ctx,
    )
    .await?;
    ctx.outcome = Some(PreflightOutcome::Ok);
    ctx.duration_ms = Some(started.elapsed().as_millis() as u64);
    let n_end = run_worker_lifecycle_event_on_guest(
        client,
        handle,
        &steps,
        PreflightLifecycleEvent::WorkerInitEnd,
        &ctx,
    )
    .await?;
    if n_start + n_end > 0 {
        info!(
            target: "claw_worker_lifecycle_preflight",
            proj_id,
            worker_id,
            n_start,
            n_end,
            "worker.init lifecycle preflight finished"
        );
    }
    Ok(())
}

/// On acquire of an existing worker: `worker.reuse.start`. Author: kejiqing
pub async fn run_worker_reuse_start_on_acquire(
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
    solve_preflight_json: &Value,
    proj_id: i64,
    worker_id: &str,
    template_id: &str,
    sandbox_id: &str,
    worker_profile_mode: &str,
    session_id: Option<&str>,
) -> Result<(), String> {
    let steps = steps_from_solve_preflight_json(solve_preflight_json);
    let mut ctx = build_worker_init_context(
        proj_id,
        worker_id,
        "/claw_ds",
        template_id,
        sandbox_id,
        worker_profile_mode,
    );
    if let Some(sid) = session_id {
        ctx.session_id = Some(sid.to_string());
    }
    let n = run_worker_lifecycle_event_on_guest(
        client,
        handle,
        &steps,
        PreflightLifecycleEvent::WorkerReuseStart,
        &ctx,
    )
    .await?;
    if n > 0 {
        info!(
            target: "claw_worker_lifecycle_preflight",
            proj_id,
            worker_id,
            n,
            "worker.reuse.start preflight finished"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use preflight_spi::PreflightScope;

    #[test]
    fn steps_parse_worker_init_on() {
        let raw = json!({
            "steps": [{
                "pluginId": "apt_tools",
                "on": "worker.init.start",
                "impl": { "type": "subprocess", "command": ["true"] }
            }, {
                "pluginId": "turn_language",
                "scope": "every_turn"
            }]
        });
        let steps = steps_from_solve_preflight_json(&raw);
        assert_eq!(steps.len(), 2);
        assert_eq!(
            steps[0].resolved_event(),
            PreflightLifecycleEvent::WorkerInitStart
        );
        let filter = PreflightFilterContext {
            is_continuation: false,
            session_first_turn_satisfied: false,
        };
        assert_eq!(
            filter_step_indices_for_event(
                &steps,
                PreflightLifecycleEvent::WorkerInitStart,
                filter
            ),
            vec![0]
        );
        assert_eq!(steps[1].scope, Some(PreflightScope::EveryTurn));
    }

    #[test]
    fn guest_script_contains_heredoc() {
        let s = guest_spi_script(&["/bin/true".into()], r#"{"spiVersion":"1"}"#);
        assert!(s.contains("CLAW_PREFLIGHT_SPI_EOF"));
        assert!(s.contains("/bin/true"));
    }
}
