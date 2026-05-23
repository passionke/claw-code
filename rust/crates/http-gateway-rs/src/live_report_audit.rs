//! Live report (stdout-v1) env audit and non-silent forward failures. Author: kejiqing

use std::sync::OnceLock;
use tracing::{error, info, warn};

/// Locked contract id exposed on `/healthz` and startup logs.
pub const LIVE_REPORT_CONTRACT: &str = "stdout-v1";

#[derive(Debug, Clone, Copy)]
pub struct LiveReportForwardEnv {
    pub base_url_ok: bool,
    pub token_ok: bool,
}

impl LiveReportForwardEnv {
    #[must_use]
    pub fn read() -> Self {
        let base_url_ok = std::env::var("CLAW_GATEWAY_INTERNAL_BASE_URL")
            .ok()
            .is_some_and(|v| !v.trim().is_empty());
        let token_ok = std::env::var("CLAW_GATEWAY_INTERNAL_TOKEN")
            .ok()
            .is_some_and(|v| !v.trim().is_empty());
        Self {
            base_url_ok,
            token_ok,
        }
    }

    #[must_use]
    pub fn ready(self) -> bool {
        self.base_url_ok && self.token_ok
    }
}

/// Pool daemon or ops: log whether HTTP forward to gateway can work.
pub fn log_live_report_startup(component: &str, pool_mode: &str) {
    let env = LiveReportForwardEnv::read();
    if env.ready() {
        info!(
            target: "claw_live_report",
            component = %component,
            contract = LIVE_REPORT_CONTRACT,
            pool_mode = %pool_mode,
            base_url = %std::env::var("CLAW_GATEWAY_INTERNAL_BASE_URL").unwrap_or_default(),
            "live_report.forward_env_ok"
        );
    } else {
        error!(
            target: "claw_live_report",
            component = %component,
            contract = LIVE_REPORT_CONTRACT,
            pool_mode = %pool_mode,
            base_url_ok = env.base_url_ok,
            token_ok = env.token_ok,
            "live_report.forward_env_missing — worker stdout report.delta will NOT reach gateway hub"
        );
    }
}

static WARN_MISSING_BASE: OnceLock<()> = OnceLock::new();
static WARN_MISSING_TOKEN: OnceLock<()> = OnceLock::new();
static WARN_CLIENT_BUILD: OnceLock<()> = OnceLock::new();

/// Called when pool daemon tries HTTP forward but env is incomplete (once per reason).
pub fn warn_forward_env_missing_once(turn_id: &str) {
    let env = LiveReportForwardEnv::read();
    if env.ready() {
        return;
    }
    if !env.base_url_ok {
        WARN_MISSING_BASE.get_or_init(|| {
            error!(
                target: "claw_live_report",
                turn_id = %turn_id,
                contract = LIVE_REPORT_CONTRACT,
                "live_report.forward_skipped — set CLAW_GATEWAY_INTERNAL_BASE_URL (host daemon: http://127.0.0.1:<GATEWAY_HOST_PORT>)"
            );
        });
    }
    if !env.token_ok {
        WARN_MISSING_TOKEN.get_or_init(|| {
            error!(
                target: "claw_live_report",
                turn_id = %turn_id,
                contract = LIVE_REPORT_CONTRACT,
                "live_report.forward_skipped — set CLAW_GATEWAY_INTERNAL_TOKEN"
            );
        });
    }
}

/// HTTP client build failed (once).
pub fn warn_forward_client_build_failed_once(turn_id: &str, error: &str) {
    WARN_CLIENT_BUILD.get_or_init(|| {
        error!(
            target: "claw_live_report",
            turn_id = %turn_id,
            contract = LIVE_REPORT_CONTRACT,
            error = %error,
            "live_report.forward_skipped — reqwest client build failed"
        );
    });
}

/// Non-prefixed stdout during solve is normal; log at debug only when line looks like a broken envelope.
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
