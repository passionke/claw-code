//! Live report (stdout-v1-pool-sse) startup audit. Author: kejiqing

use tracing::{info, warn};

/// Locked contract id exposed on `/healthz` and startup logs.
pub const LIVE_REPORT_CONTRACT: &str = "stdout-v1-pool-sse";

/// Pool daemon startup: log live ingest mode.
pub fn log_live_report_startup(component: &str, pool_mode: &str) {
    info!(
        target: "claw_live_report",
        component = %component,
        contract = LIVE_REPORT_CONTRACT,
        pool_mode = %pool_mode,
        ingest = "pool-local",
        "live_report.startup"
    );
}

/// Non-prefixed stdout during solve is normal; log when line looks like a broken envelope.
pub fn debug_non_claw_stdout_line(turn_id: &str, line: &str) {
    let t = line.trim();
    if t.contains("__CLAW_GATEWAY_STDOUT__") || t.contains("report.delta") {
        warn!(
            target: "claw_live_report",
            turn_id = %turn_id,
            line_prefix = %t.chars().take(120).collect::<String>(),
            "live_report.stdout_line_unparsed — check worker binary emits valid JSON after prefix"
        );
    }
}
