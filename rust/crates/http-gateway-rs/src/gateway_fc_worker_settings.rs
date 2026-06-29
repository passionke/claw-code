//! FC worker template id managed by gateway (rotation per proj). Author: kejiqing

use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;

/// Gateway-desired claw-worker e2b template (`settings_json.fcWorker.templateId`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FcWorkerSettings {
    #[serde(rename = "templateId", default)]
    pub template_id: Option<String>,
    #[serde(rename = "updatedAtMs", default)]
    pub updated_at_ms: i64,
}

impl FcWorkerSettings {
    #[must_use]
    pub fn configured(&self) -> bool {
        self.template_id
            .as_ref()
            .is_some_and(|t| !t.trim().is_empty())
    }
}

/// Effective worker template: PG `fcWorker.templateId` → env `CLAW_FC_TEMPLATE` → `claw-worker`.
#[must_use]
pub fn fc_worker_template_from_env() -> String {
    std::env::var("CLAW_FC_TEMPLATE")
        .ok()
        .or_else(|| std::env::var("CLAW_E2B_TEMPLATE").ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "claw-worker".into())
}

/// Project worker e2b TTL on each renew (`CLAW_FC_PROJECT_WORKER_TTL_SECS` → `CLAW_FC_SANDBOX_TIMEOUT_SECS` → 3600).
#[must_use]
pub fn fc_project_worker_ttl_secs_from_env() -> u64 {
    parse_positive_u64_env("CLAW_FC_PROJECT_WORKER_TTL_SECS")
        .or_else(|| parse_positive_u64_env("CLAW_FC_SANDBOX_TIMEOUT_SECS"))
        .unwrap_or(3600)
}

/// Background reconcile tick (`CLAW_FC_PROJECT_WORKER_RENEW_INTERVAL_SECS` or 600s).
/// TTL touch uses [`claw_fc_sandbox_client::SANDBOX_LEASE_TICK_SECS`] lease ticker.
#[must_use]
pub fn fc_project_worker_renew_interval_secs_from_env(_ttl_secs: u64) -> u64 {
    parse_positive_u64_env("CLAW_FC_PROJECT_WORKER_RENEW_INTERVAL_SECS").unwrap_or(600)
}

fn parse_positive_u64_env(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
}

pub async fn load_fc_worker_template_id(db: &GatewaySessionDb) -> Result<String, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings
        .fc_worker
        .template_id
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(fc_worker_template_from_env))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renew_interval_defaults_to_ten_minutes() {
        assert_eq!(fc_project_worker_renew_interval_secs_from_env(3600), 600);
        assert_eq!(fc_project_worker_renew_interval_secs_from_env(31_536_000), 600);
    }
}
