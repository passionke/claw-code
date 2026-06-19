//! Pool-managed claude-tap: gateway probes/injects URL only (no tap-up.sh). Author: kejiqing

use serde::Serialize;

use crate::gateway_claw_tap_settings::{ClawTapMode, ClawTapSettings};
use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Serialize)]
pub struct TapRestartOutcome {
    pub attempted: bool,
    pub restarted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[must_use]
pub fn is_local_claw_tap(settings: &ClawTapSettings) -> bool {
    settings.mode == ClawTapMode::Local
}

const POOL_MANAGED_TAP_HINT: &str = "claude-tap is pool-managed (gateway.sh up / pool-up); upstream hot-reloads via claw-tap-upstream.json";

/// Gateway does not restart tap; pool host scripts own lifecycle.
pub async fn restart_local_claw_tap(_live_port: u16) -> TapRestartOutcome {
    TapRestartOutcome {
        attempted: false,
        restarted: false,
        message: Some(POOL_MANAGED_TAP_HINT.into()),
    }
}

pub async fn restart_local_claw_tap_if_configured(db: &GatewaySessionDb) -> TapRestartOutcome {
    let settings = match get_gateway_global_settings(db).await {
        Ok((s, _, _)) => s.claw_tap,
        Err(e) => {
            return TapRestartOutcome {
                attempted: false,
                restarted: false,
                message: Some(format!("load clawTap settings: {e}")),
            };
        }
    };
    if !is_local_claw_tap(&settings) {
        return TapRestartOutcome {
            attempted: false,
            restarted: false,
            message: None,
        };
    }
    restart_local_claw_tap(settings.live_port).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn restart_local_claw_tap_is_pool_managed_noop() {
        let out = restart_local_claw_tap(3000).await;
        assert!(!out.attempted);
        assert!(!out.restarted);
        assert!(out
            .message
            .as_deref()
            .is_some_and(|m| m.contains("pool-managed")));
    }
}
