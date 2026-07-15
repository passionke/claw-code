//! e2b worker template id managed by gateway (rotation per proj). Author: kejiqing

use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::session_db::GatewaySessionDb;

/// Default strict worker pool size per project (Admin `e2bWorker.poolSize`).
pub const STRICT_WORKER_POOL_SIZE_DEFAULT: u32 = 1;
/// Fallback upper bound when `CLAW_E2B_POOL_SIZE_CAP` unset.
pub const STRICT_WORKER_POOL_SIZE_MAX: u32 = 16;

/// Admin / runtime upper bound (`CLAW_E2B_POOL_SIZE_CAP` → default [`STRICT_WORKER_POOL_SIZE_MAX`]).
#[must_use]
pub fn strict_worker_pool_size_cap_from_env() -> u32 {
    std::env::var("CLAW_E2B_POOL_SIZE_CAP")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|&n| n >= 1)
        .unwrap_or(STRICT_WORKER_POOL_SIZE_MAX)
}

/// Reject out-of-range poolSize (Admin write — do not silently clamp). Author: kejiqing
pub fn validate_strict_worker_pool_size(n: u32) -> Result<u32, String> {
    let max = strict_worker_pool_size_cap_from_env();
    if !(1..=max).contains(&n) {
        return Err(format!(
            "poolSize must be 1..={max} (CLAW_E2B_POOL_SIZE_CAP); got {n}"
        ));
    }
    Ok(n)
}

/// Runtime clamp (legacy PG rows may exceed a later-lowered env cap).
#[must_use]
pub fn clamp_strict_worker_pool_size(n: u32) -> u32 {
    n.clamp(1, strict_worker_pool_size_cap_from_env())
}

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
    /// Env `CLAW_E2B_POOL_SIZE_CAP` (Admin write rejects values above this).
    #[serde(rename = "poolSizeCap")]
    pub pool_size_cap: u32,
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
        pool_size_cap: strict_worker_pool_size_cap_from_env(),
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
        settings.e2b_worker.pool_size = Some(validate_strict_worker_pool_size(n)?);
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

/// Background reconcile tick (`CLAW_E2B_PROJECT_WORKER_RENEW_INTERVAL_SECS` or 600s):
/// best-effort TTL touch for persisted sandboxes (full `reconcile_proj` is startup / Admin only).
/// Primary TTL renewal: [`claw_e2b_sandbox_client::SANDBOX_LEASE_TICK_SECS`] lease ticker.
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

/// PG `e2bWorker.poolSize` → default 1, clamped to 1..=`CLAW_E2B_POOL_SIZE_CAP`.
pub async fn load_e2b_strict_worker_pool_size(db: &GatewaySessionDb) -> Result<u32, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(clamp_strict_worker_pool_size(
        settings
            .e2b_worker
            .pool_size
            .unwrap_or(STRICT_WORKER_POOL_SIZE_DEFAULT),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::test_env_lock;

    fn with_env(key: &str, value: Option<&str>, f: impl FnOnce()) {
        let _guard = test_env_lock();
        let prev = std::env::var(key).ok();
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        f();
        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    #[test]
    fn default_pool_size_is_one() {
        assert_eq!(STRICT_WORKER_POOL_SIZE_DEFAULT, 1);
        let public = e2b_worker_settings_public(&E2bWorkerSettings::default());
        assert_eq!(public.pool_size, 1);
    }

    #[test]
    fn pool_size_clamps_to_bounds() {
        with_env("CLAW_E2B_POOL_SIZE_CAP", None, || {
            assert_eq!(clamp_strict_worker_pool_size(0), 1);
            assert_eq!(clamp_strict_worker_pool_size(1), 1);
            assert_eq!(
                clamp_strict_worker_pool_size(STRICT_WORKER_POOL_SIZE_MAX),
                STRICT_WORKER_POOL_SIZE_MAX
            );
            assert_eq!(
                clamp_strict_worker_pool_size(STRICT_WORKER_POOL_SIZE_MAX + 99),
                STRICT_WORKER_POOL_SIZE_MAX
            );
        });
    }

    #[test]
    fn pool_size_validate_rejects_over_cap() {
        with_env("CLAW_E2B_POOL_SIZE_CAP", Some("32"), || {
            assert_eq!(strict_worker_pool_size_cap_from_env(), 32);
            assert!(validate_strict_worker_pool_size(0).is_err());
            assert!(validate_strict_worker_pool_size(1).is_ok());
            assert!(validate_strict_worker_pool_size(32).is_ok());
            let err = validate_strict_worker_pool_size(33).unwrap_err();
            assert!(
                err.contains("CLAW_E2B_POOL_SIZE_CAP"),
                "unexpected err: {err}"
            );
        });
    }

    #[test]
    fn pool_size_cap_falls_back_when_unset_or_invalid() {
        with_env("CLAW_E2B_POOL_SIZE_CAP", None, || {
            assert_eq!(
                strict_worker_pool_size_cap_from_env(),
                STRICT_WORKER_POOL_SIZE_MAX
            );
        });
        with_env("CLAW_E2B_POOL_SIZE_CAP", Some("0"), || {
            assert_eq!(
                strict_worker_pool_size_cap_from_env(),
                STRICT_WORKER_POOL_SIZE_MAX
            );
        });
        with_env("CLAW_E2B_POOL_SIZE_CAP", Some("nope"), || {
            assert_eq!(
                strict_worker_pool_size_cap_from_env(),
                STRICT_WORKER_POOL_SIZE_MAX
            );
        });
    }

    #[test]
    fn public_settings_exposes_pool_size_cap() {
        with_env("CLAW_E2B_POOL_SIZE_CAP", Some("8"), || {
            let public = e2b_worker_settings_public(&E2bWorkerSettings {
                pool_size: Some(4),
                ..Default::default()
            });
            assert_eq!(public.pool_size, 4);
            assert_eq!(public.pool_size_cap, 8);
        });
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
