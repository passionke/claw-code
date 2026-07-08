//! Unified e2b core singleton readiness (nas-api + observe + clawTap cluster). Author: kejiqing

use serde::Serialize;

use crate::claw_tap_cluster_state::{self, ClawTapClusterHandle, ClawTapClusterSnapshot};
use crate::gateway_e2b_nas_api_settings::{
    e2b_nas_api_settings_public, e2b_nas_api_settings_public_with_runtime, E2bNasApiSettingsPublic,
};
use crate::gateway_e2b_observe_settings::{
    e2b_observe_settings_public, e2b_observe_settings_public_with_runtime, E2bObserveSettingsPublic,
};
use crate::pool::interactive_backend::{e2b_observe_is_enabled, E2bNasApiSingleton};
use crate::session_db::GatewaySessionDb;
use claw_e2b_sandbox_client::E2bSandboxClient;

#[derive(Debug, Clone, Serialize)]
pub struct E2bCoreReadinessSnapshot {
    pub ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "nasApi")]
    pub nas_api: E2bNasApiSettingsPublic,
    pub observe: E2bObserveSettingsPublic,
    #[serde(rename = "clawTapCluster")]
    pub claw_tap_cluster: ClawTapClusterSnapshot,
}

/// nas-api is required when `CLAW_E2B_NAS_API` is enabled (default in e2b mode).
#[must_use]
pub fn nas_api_component_ready(s: &E2bNasApiSettingsPublic) -> bool {
    if !E2bNasApiSingleton::enabled_from_env() {
        return true;
    }
    s.online
}

/// observe singleton readiness when e2b observe is enabled.
#[must_use]
pub fn observe_component_ready(s: &E2bObserveSettingsPublic) -> bool {
    if !e2b_observe_is_enabled() {
        return true;
    }
    s.healthy
}

/// Aggregate core readiness from component snapshots (unit-testable).
#[must_use]
pub fn aggregate_core_ready(
    nas_api: &E2bNasApiSettingsPublic,
    observe: &E2bObserveSettingsPublic,
    claw_tap: &ClawTapClusterSnapshot,
) -> bool {
    nas_api_component_ready(nas_api)
        && observe_component_ready(observe)
        && claw_tap_cluster_state::is_ready(claw_tap)
}

fn first_blocking_reason(
    nas_api: &E2bNasApiSettingsPublic,
    observe: &E2bObserveSettingsPublic,
    claw_tap: &ClawTapClusterSnapshot,
) -> Option<String> {
    if E2bNasApiSingleton::enabled_from_env() && !nas_api_component_ready(nas_api) {
        return nas_api
            .last_error
            .clone()
            .or_else(|| Some("nas-api not online".into()));
    }
    if e2b_observe_is_enabled() && !observe_component_ready(observe) {
        return observe
            .last_error
            .clone()
            .or_else(|| Some("observe not healthy".into()));
    }
    if !claw_tap_cluster_state::is_ready(claw_tap) {
        return claw_tap
            .reason
            .clone()
            .or_else(|| Some("clawTap cluster not strict".into()));
    }
    None
}

pub async fn load_core_readiness_snapshot(
    db: &GatewaySessionDb,
    client: Option<&E2bSandboxClient>,
    claw_tap_cluster: &ClawTapClusterHandle,
) -> Result<E2bCoreReadinessSnapshot, sqlx::Error> {
    let nas_api = match client {
        Some(c) => e2b_nas_api_settings_public_with_runtime(db, Some(c)).await?,
        None => e2b_nas_api_settings_public(db).await?,
    };
    let observe = match client {
        Some(c) => e2b_observe_settings_public_with_runtime(db, Some(c)).await?,
        None => e2b_observe_settings_public(db).await?,
    };
    let claw_tap_cluster_snap =
        claw_tap_cluster_state::snapshot_from_handle(claw_tap_cluster).await;
    let ready = aggregate_core_ready(&nas_api, &observe, &claw_tap_cluster_snap);
    let reason = if ready {
        None
    } else {
        first_blocking_reason(&nas_api, &observe, &claw_tap_cluster_snap)
    };
    Ok(E2bCoreReadinessSnapshot {
        ready,
        reason,
        nas_api,
        observe,
        claw_tap_cluster: claw_tap_cluster_snap,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claw_tap_cluster_state::{ClawTapClusterSnapshot, TapConsistency};

    fn nas_api_missing_base() -> E2bNasApiSettingsPublic {
        E2bNasApiSettingsPublic {
            template_id: None,
            effective_template_id: "claw-nas-api".into(),
            base_url: None,
            sandbox_id: None,
            updated_at_ms: 0,
            configured: false,
            running: None,
            reachable: false,
            healthy: false,
            last_checked_at_ms: None,
            last_error: Some("baseUrl not configured".into()),
            online: false,
        }
    }

    fn observe_missing_live() -> E2bObserveSettingsPublic {
        E2bObserveSettingsPublic {
            template_id: None,
            effective_template_id: "claw-observe".into(),
            updated_at_ms: 0,
            configured: true,
            base_url: None,
            sandbox_id: None,
            running: None,
            reachable: false,
            healthy: false,
            last_checked_at_ms: None,
            last_error: Some("observe liveBaseUrl not configured".into()),
        }
    }

    fn tap_unconfigured() -> ClawTapClusterSnapshot {
        ClawTapClusterSnapshot {
            cluster_id: None,
            tap_base_url: None,
            consistency: TapConsistency::Unconfigured,
            reason: Some("CLAW_CLUSTER_ID or clawTap not configured".into()),
            last_check_ms: None,
            local_cluster_hash: None,
            tap_cluster_hash: None,
        }
    }

    #[test]
    fn aggregate_not_ready_when_nas_api_missing_base_url() {
        let nas = nas_api_missing_base();
        let obs = observe_missing_live();
        let tap = tap_unconfigured();
        assert!(!aggregate_core_ready(&nas, &obs, &tap));
        let reason = first_blocking_reason(&nas, &obs, &tap).unwrap_or_default();
        assert!(reason.contains("baseUrl not configured"));
    }

    #[test]
    fn aggregate_not_ready_when_observe_missing_live_base_url() {
        let nas = E2bNasApiSettingsPublic {
            online: true,
            healthy: true,
            configured: true,
            effective_template_id: "claw-nas-api".into(),
            base_url: Some("http://8090-sbx_test.spone.xyz".into()),
            sandbox_id: Some("sbx_test".into()),
            updated_at_ms: 1,
            template_id: None,
            running: Some(true),
            reachable: true,
            last_checked_at_ms: Some(1),
            last_error: None,
        };
        let obs = observe_missing_live();
        let tap = tap_unconfigured();
        assert!(!aggregate_core_ready(&nas, &obs, &tap));
        let reason = first_blocking_reason(&nas, &obs, &tap).unwrap_or_default();
        assert!(reason.contains("liveBaseUrl not configured"));
    }

    #[test]
    fn aggregate_ready_when_all_components_healthy() {
        let nas = E2bNasApiSettingsPublic {
            online: true,
            healthy: true,
            configured: true,
            effective_template_id: "claw-nas-api".into(),
            base_url: Some("http://8090-sbx_test.spone.xyz".into()),
            sandbox_id: Some("sbx_test".into()),
            updated_at_ms: 1,
            template_id: None,
            running: Some(true),
            reachable: true,
            last_checked_at_ms: Some(1),
            last_error: None,
        };
        let obs = E2bObserveSettingsPublic {
            template_id: None,
            effective_template_id: "claw-observe".into(),
            updated_at_ms: 1,
            configured: true,
            base_url: Some("http://3000-sbx_o.spone.xyz".into()),
            sandbox_id: Some("sbx_o".into()),
            running: Some(true),
            reachable: true,
            healthy: true,
            last_checked_at_ms: Some(1),
            last_error: None,
        };
        let tap = ClawTapClusterSnapshot {
            cluster_id: Some("local-dev".into()),
            tap_base_url: Some("http://8080-sbx_o.spone.xyz".into()),
            consistency: TapConsistency::Strict,
            reason: None,
            last_check_ms: Some(1),
            local_cluster_hash: Some("sha256:abc".into()),
            tap_cluster_hash: Some("sha256:abc".into()),
        };
        assert!(aggregate_core_ready(&nas, &obs, &tap));
        assert!(first_blocking_reason(&nas, &obs, &tap).is_none());
    }
}
