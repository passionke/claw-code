//! e2b worker template id managed by gateway (rotation per proj). Author: kejiqing

use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::session_db::GatewaySessionDb;

/// Default strict worker pool size per project (Admin `e2bWorker.poolSize`).
pub const STRICT_WORKER_POOL_SIZE_DEFAULT: u32 = 4;
/// Upper bound for Admin `e2bWorker.poolSize`.
pub const STRICT_WORKER_POOL_SIZE_MAX: u32 = 16;

/// Gateway-desired claw-worker e2b template (`settings_json.e2bWorker.templateId`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct E2bWorkerSettings {
    #[serde(rename = "templateId", default)]
    pub template_id: Option<String>,
    #[serde(rename = "poolSize", default)]
    pub pool_size: Option<u32>,
    #[serde(rename = "alias", default)]
    pub alias: Option<String>,
    #[serde(rename = "updatedAtMs", default)]
    pub updated_at_ms: i64,
}

impl E2bWorkerSettings {
    #[must_use]
    pub fn configured(&self) -> bool {
        self.template_id
            .as_ref()
            .is_some_and(|t| !t.trim().is_empty())
    }
}

/// Admin read model for `settings_json.e2bWorker`.
#[derive(Debug, Clone, Serialize)]
pub struct E2bWorkerSettingsPublic {
    #[serde(rename = "templateId", skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(rename = "poolSize")]
    pub pool_size: u32,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
}

#[derive(Debug, Deserialize)]
pub struct PutE2bWorkerSettingsInput {
    #[serde(rename = "templateId", default)]
    pub template_id: Option<String>,
    #[serde(rename = "poolSize", default)]
    pub pool_size: Option<u32>,
}

#[must_use]
pub fn e2b_worker_settings_public(settings: &E2bWorkerSettings) -> E2bWorkerSettingsPublic {
    E2bWorkerSettingsPublic {
        template_id: settings
            .template_id
            .clone()
            .filter(|t| !t.trim().is_empty()),
        pool_size: clamp_strict_worker_pool_size(
            settings
                .pool_size
                .unwrap_or(STRICT_WORKER_POOL_SIZE_DEFAULT),
        ),
        updated_at_ms: settings.updated_at_ms,
    }
}

pub async fn put_e2b_worker_settings(
    db: &GatewaySessionDb,
    input: PutE2bWorkerSettingsInput,
) -> Result<E2bWorkerSettingsPublic, String> {
    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| format!("load global settings: {e}"))?;
    if let Some(tpl) = input.template_id {
        let trimmed = tpl.trim();
        if trimmed.is_empty() {
            settings.e2b_worker.template_id = None;
        } else {
            settings.e2b_worker.template_id = Some(trimmed.to_string());
        }
    }
    if let Some(n) = input.pool_size {
        settings.e2b_worker.pool_size = Some(clamp_strict_worker_pool_size(n));
    } else if settings.e2b_worker.pool_size.is_none() {
        settings.e2b_worker.pool_size = Some(STRICT_WORKER_POOL_SIZE_DEFAULT);
    }
    let now = chrono::Utc::now().timestamp_millis();
    settings.e2b_worker.updated_at_ms = now;
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| format!("save global settings: {e}"))?;
    Ok(e2b_worker_settings_public(&settings.e2b_worker))
}

/// Effective strict worker template: PG `e2bWorker.templateId` → env `CLAW_E2B_TEMPLATE` → `claw-worker`.
#[must_use]
pub fn e2b_worker_template_from_env() -> String {
    std::env::var("CLAW_E2B_TEMPLATE")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "claw-worker".into())
}

/// Relaxed fallback: env `CLAW_E2B_WORKER_RELAXED_TEMPLATE` → alias `claw-worker-relaxed`.
#[must_use]
pub fn e2b_worker_relaxed_template_from_env() -> String {
    std::env::var("CLAW_E2B_WORKER_RELAXED_TEMPLATE")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "claw-worker-relaxed".into())
}

/// Browser → gateway WS (`claw.gatewayPublicHost` in OVS settings).
#[must_use]
pub fn ovs_gateway_public_host() -> String {
    if let Ok(v) = std::env::var("CLAW_GATEWAY_PUBLIC_HOST") {
        let t = v.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    let port = std::env::var("GATEWAY_HOST_PORT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "8088".into());
    if let Ok(host) = std::env::var("CLAW_POOL_ADVERTISE_HOST") {
        let h = host.trim();
        if !h.is_empty() {
            return format!("{h}:{port}");
        }
    }
    format!("127.0.0.1:{port}")
}

/// e2b OVS sandbox → gateway HTTP/WS (`claw.gatewayHost`; reachable from worker sandbox).
#[must_use]
pub fn ovs_gateway_host_for_e2b() -> String {
    if let Ok(v) = std::env::var("CLAW_E2B_OVS_GATEWAY_HOST") {
        let t = v.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    if let Ok(v) = std::env::var("CLAW_E2B_GATEWAY_ADVERTISE_HOST") {
        let t = v.trim();
        if !t.is_empty() {
            return if t.contains(':') {
                t.to_string()
            } else {
                let port = std::env::var("GATEWAY_HOST_PORT")
                    .ok()
                    .filter(|p| !p.trim().is_empty())
                    .unwrap_or_else(|| "8088".into());
                format!("{t}:{port}")
            };
        }
    }
    ovs_gateway_public_host()
}

/// Project worker e2b TTL on each renew (`CLAW_E2B_PROJECT_WORKER_TTL_SECS` → `CLAW_E2B_SANDBOX_TIMEOUT_SECS` → 3600).
#[must_use]
pub fn e2b_project_worker_ttl_secs_from_env() -> u64 {
    parse_positive_u64_env("CLAW_E2B_PROJECT_WORKER_TTL_SECS")
        .or_else(|| parse_positive_u64_env("CLAW_E2B_SANDBOX_TIMEOUT_SECS"))
        .unwrap_or(3600)
}

/// Background reconcile tick (`CLAW_E2B_PROJECT_WORKER_RENEW_INTERVAL_SECS` or 600s).
/// TTL touch uses [`claw_e2b_sandbox_client::SANDBOX_LEASE_TICK_SECS`] lease ticker.
#[must_use]
pub fn e2b_project_worker_renew_interval_secs_from_env(_ttl_secs: u64) -> u64 {
    parse_positive_u64_env("CLAW_E2B_PROJECT_WORKER_RENEW_INTERVAL_SECS").unwrap_or(600)
}

fn parse_positive_u64_env(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
}

pub async fn load_e2b_worker_template_id(db: &GatewaySessionDb) -> Result<String, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings
        .e2b_worker
        .template_id
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(e2b_worker_template_from_env))
}

/// PG `e2bWorkerRelaxed.templateId` → env → alias `claw-worker-relaxed`.
pub async fn load_e2b_worker_relaxed_template_id(
    db: &GatewaySessionDb,
) -> Result<String, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings
        .e2b_worker_relaxed
        .template_id
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(e2b_worker_relaxed_template_from_env))
}

/// PG `e2bWorker.poolSize` → default 4, clamped to 1..=16.
pub async fn load_e2b_strict_worker_pool_size(db: &GatewaySessionDb) -> Result<u32, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(clamp_strict_worker_pool_size(
        settings
            .e2b_worker
            .pool_size
            .unwrap_or(STRICT_WORKER_POOL_SIZE_DEFAULT),
    ))
}

#[must_use]
pub fn clamp_strict_worker_pool_size(n: u32) -> u32 {
    n.clamp(1, STRICT_WORKER_POOL_SIZE_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_size_clamps_to_bounds() {
        assert_eq!(clamp_strict_worker_pool_size(0), 1);
        assert_eq!(clamp_strict_worker_pool_size(4), 4);
        assert_eq!(
            clamp_strict_worker_pool_size(99),
            STRICT_WORKER_POOL_SIZE_MAX
        );
    }

    #[test]
    fn renew_interval_defaults_to_ten_minutes() {
        assert_eq!(e2b_project_worker_renew_interval_secs_from_env(3600), 600);
        assert_eq!(
            e2b_project_worker_renew_interval_secs_from_env(31_536_000),
            600
        );
    }
}
