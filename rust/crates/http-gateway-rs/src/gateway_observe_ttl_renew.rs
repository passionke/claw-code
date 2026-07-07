//! Observe singleton TTL renew ticker (e2b only). Author: kejiqing
//!
//! Dedicated env keys (do not reuse `CLAW_E2B_SANDBOX_TIMEOUT_SECS`):
//! - `CLAW_OBSERVE_TTL_RENEW_POLL_INTERVAL_SECS` — default 60
//! - `CLAW_OBSERVE_TTL_RENEW_THRESHOLD_SECS` — renew when remaining ≤ this; default 600 (10m)
//! - `CLAW_OBSERVE_TTL_RENEW_EXTEND_SECS` — POST /timeout value; default 3600 (1h)

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use claw_e2b_sandbox_client::E2bSandboxClient;
use tracing::{info, warn};

use crate::gateway_global_settings::get_gateway_global_settings;
use crate::pool::interactive_backend::interactive_backend_is_e2b;
use crate::session_db::GatewaySessionDb;

const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
const DEFAULT_THRESHOLD_SECS: u64 = 600;
const DEFAULT_EXTEND_SECS: u64 = 3600;

const ENV_POLL_INTERVAL: &str = "CLAW_OBSERVE_TTL_RENEW_POLL_INTERVAL_SECS";
const ENV_THRESHOLD: &str = "CLAW_OBSERVE_TTL_RENEW_THRESHOLD_SECS";
const ENV_EXTEND: &str = "CLAW_OBSERVE_TTL_RENEW_EXTEND_SECS";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn parse_positive_u64_env(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
}

#[must_use]
pub fn observe_ttl_renew_poll_interval_secs() -> u64 {
    parse_positive_u64_env(ENV_POLL_INTERVAL).unwrap_or(DEFAULT_POLL_INTERVAL_SECS)
}

#[must_use]
pub fn observe_ttl_renew_threshold_secs() -> u64 {
    parse_positive_u64_env(ENV_THRESHOLD).unwrap_or(DEFAULT_THRESHOLD_SECS)
}

#[must_use]
pub fn observe_ttl_renew_extend_secs() -> u64 {
    parse_positive_u64_env(ENV_EXTEND).unwrap_or(DEFAULT_EXTEND_SECS)
}

/// One renew pass: extend observe sandbox TTL when remaining ≤ threshold.
pub async fn observe_ttl_renew_once(
    client: &E2bSandboxClient,
    db: &GatewaySessionDb,
) -> Result<(), String> {
    let (settings, _, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| format!("load gateway settings: {e}"))?;
    let sandbox_id = settings
        .claw_tap
        .e2b_observe_sandbox_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "clawTap.e2bObserveSandboxId not configured".to_string())?;

    let snap = client.fetch_sandbox_snapshot(sandbox_id).await?;
    if !snap.is_running() {
        return Err(format!(
            "observe sandbox {sandbox_id} not running (state={})",
            snap.state
        ));
    }

    let remaining = snap.remaining_ttl_secs(now_ms()).unwrap_or(0);
    let threshold = observe_ttl_renew_threshold_secs();
    if remaining > threshold {
        return Ok(());
    }

    let extend_secs = observe_ttl_renew_extend_secs();
    let renewed = client
        .renew_sandbox_ttl_verified(sandbox_id, extend_secs)
        .await?;
    let new_remaining = renewed.remaining_ttl_secs(now_ms()).unwrap_or(0);
    info!(
        target: "claw_gateway_orchestration",
        component = "observe_ttl_renew",
        sandbox_id,
        previous_remaining_secs = remaining,
        threshold_secs = threshold,
        extend_secs,
        new_remaining_secs = new_remaining,
        "observe singleton TTL extended"
    );
    Ok(())
}

pub async fn observe_ttl_renew_loop(client: Arc<E2bSandboxClient>, db: Arc<GatewaySessionDb>) {
    if !interactive_backend_is_e2b() {
        return;
    }
    let poll_secs = observe_ttl_renew_poll_interval_secs();
    if poll_secs == 0 {
        return;
    }
    info!(
        target: "claw_gateway_orchestration",
        component = "observe_ttl_renew",
        poll_interval_secs = poll_secs,
        threshold_secs = observe_ttl_renew_threshold_secs(),
        extend_secs = observe_ttl_renew_extend_secs(),
        "observe singleton TTL renew ticker enabled"
    );
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(poll_secs));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if let Err(e) = observe_ttl_renew_once(client.as_ref(), db.as_ref()).await {
            warn!(
                target: "claw_gateway_orchestration",
                component = "observe_ttl_renew",
                error = %e,
                "observe TTL renew tick failed"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_ttl_renew_defaults_are_observe_specific() {
        assert_eq!(observe_ttl_renew_poll_interval_secs(), 60);
        assert_eq!(observe_ttl_renew_threshold_secs(), 600);
        assert_eq!(observe_ttl_renew_extend_secs(), 3600);
    }
}
