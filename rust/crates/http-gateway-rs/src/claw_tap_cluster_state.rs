//! In-memory clawTap cluster consistency + solve LLM routing. Author: kejiqing

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::cluster_identity::{
    fetch_tap_cluster_identity, gateway_cluster_id, gateway_database_url, local_cluster_identity,
    verify_tap_cluster, ClusterIdentity, ClusterMismatchError,
};
use crate::gateway_claw_tap_settings::{claw_tap_proxy_base_url, ClawTapSettings};
use crate::gateway_global_settings::{self, ActiveLlmRuntime};
use crate::gateway_llm_config_sync::LlmRuntimeHandle;
use crate::gateway_llm_model_apply::{
    normalize_model_name_for_upstream, normalize_upstream_base_url,
};
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TapConsistency {
    /// clawTap health clusterId + clusterHash match gateway PG.
    Strict,
    /// `CLAW_CLUSTER_ID` unset or Admin clawTap not configured.
    Unconfigured,
    /// Runtime poll: tap unreachable or cluster identity mismatch (solve blocked).
    ClusterMismatch,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClawTapClusterSnapshot {
    #[serde(rename = "clusterId", skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    #[serde(rename = "tapBaseUrl", skip_serializing_if = "Option::is_none")]
    pub tap_base_url: Option<String>,
    pub consistency: TapConsistency,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "lastCheckMs", skip_serializing_if = "Option::is_none")]
    pub last_check_ms: Option<i64>,
    #[serde(rename = "localClusterHash", skip_serializing_if = "Option::is_none")]
    pub local_cluster_hash: Option<String>,
    #[serde(rename = "tapClusterHash", skip_serializing_if = "Option::is_none")]
    pub tap_cluster_hash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClawTapClusterStateInner {
    pub cluster_id: String,
    pub tap: ClawTapSettings,
    pub tap_base_url: String,
    pub local_identity: ClusterIdentity,
    pub consistency: TapConsistency,
    pub mismatch_reason: Option<String>,
    pub last_check_ms: i64,
    pub tap_identity: Option<ClusterIdentity>,
}

pub type ClawTapClusterHandle = Arc<RwLock<Option<ClawTapClusterStateInner>>>;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

pub fn cluster_poll_interval_secs() -> u64 {
    std::env::var("CLAW_TAP_CLUSTER_POLL_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(30)
}

pub async fn load_cluster_settings(
    db: &GatewaySessionDb,
) -> Result<(String, ClawTapSettings), String> {
    let (settings, _, _) = gateway_global_settings::get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let cluster_id = gateway_cluster_id()?;
    if settings.claw_tap.updated_at_ms <= 0 {
        return Err("clawTap endpoint not configured in Admin".into());
    }
    if settings.claw_tap.host.trim().is_empty() {
        return Err("clawTap host is required".into());
    }
    Ok((cluster_id, settings.claw_tap))
}

pub async fn refresh_claw_tap_cluster_state(
    db: &GatewaySessionDb,
    llm_handle: &LlmRuntimeHandle,
) -> Result<Option<ClawTapClusterStateInner>, String> {
    let (cluster_id, tap) = load_cluster_settings(db).await?;
    let tap_base = claw_tap_proxy_base_url(&tap.host, tap.proxy_port)
        .ok_or_else(|| "invalid clawTap host/port".to_string())?;
    let db_url = gateway_database_url()?;
    let local = local_cluster_identity(&cluster_id, &db_url)?;
    let _ = crate::gateway_llm_config_sync::sync_llm_runtime_from_db(db, llm_handle).await;
    let poll = fetch_tap_cluster_identity(&tap_base, &cluster_id).await;
    let (consistency, mismatch_reason, tap_identity) = match poll {
        Ok(tap_id) => match verify_tap_cluster(&local, &tap_id) {
            Ok(()) => (TapConsistency::Strict, None, Some(tap_id)),
            Err(ClusterMismatchError { message, .. }) => {
                (TapConsistency::ClusterMismatch, Some(message), Some(tap_id))
            }
        },
        Err(e) => (TapConsistency::ClusterMismatch, Some(e), None),
    };
    Ok(Some(ClawTapClusterStateInner {
        cluster_id,
        tap,
        tap_base_url: tap_base,
        local_identity: local,
        consistency,
        mismatch_reason,
        last_check_ms: now_ms(),
        tap_identity,
    }))
}

pub async fn snapshot_from_handle(handle: &ClawTapClusterHandle) -> ClawTapClusterSnapshot {
    let guard = handle.read().await;
    let Some(inner) = guard.as_ref() else {
        return ClawTapClusterSnapshot {
            cluster_id: None,
            tap_base_url: None,
            consistency: TapConsistency::Unconfigured,
            reason: Some("CLAW_CLUSTER_ID or clawTap not configured".into()),
            last_check_ms: None,
            local_cluster_hash: None,
            tap_cluster_hash: None,
        };
    };
    ClawTapClusterSnapshot {
        cluster_id: Some(inner.cluster_id.clone()),
        tap_base_url: Some(inner.tap_base_url.clone()),
        consistency: inner.consistency,
        reason: inner.mismatch_reason.clone(),
        last_check_ms: Some(inner.last_check_ms),
        local_cluster_hash: Some(inner.local_identity.cluster_hash.clone()),
        tap_cluster_hash: inner.tap_identity.as_ref().map(|t| t.cluster_hash.clone()),
    }
}

pub fn is_ready(snapshot: &ClawTapClusterSnapshot) -> bool {
    snapshot.consistency == TapConsistency::Strict
        && snapshot
            .cluster_id
            .as_deref()
            .is_some_and(|s| !s.is_empty())
        && snapshot
            .tap_base_url
            .as_deref()
            .is_some_and(|s| !s.is_empty())
}

pub async fn cluster_poll_loop(
    db: Arc<GatewaySessionDb>,
    llm_handle: LlmRuntimeHandle,
    handle: ClawTapClusterHandle,
) {
    let interval = cluster_poll_interval_secs();
    if interval == 0 {
        return;
    }
    if let Ok(Some(state)) = refresh_claw_tap_cluster_state(db.as_ref(), &llm_handle).await {
        *handle.write().await = Some(state);
    }
    let start = tokio::time::Instant::now() + std::time::Duration::from_secs(interval);
    let mut ticker = tokio::time::interval_at(start, std::time::Duration::from_secs(interval));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        match refresh_claw_tap_cluster_state(db.as_ref(), &llm_handle).await {
            Ok(Some(state)) => {
                if state.consistency == TapConsistency::Strict {
                    info!(
                        target: "claw_gateway_orchestration",
                        component = "claw_tap_cluster",
                        tap = %state.tap_base_url,
                        "clawTap cluster strict"
                    );
                } else {
                    warn!(
                        target: "claw_gateway_orchestration",
                        component = "claw_tap_cluster",
                        consistency = ?state.consistency,
                        reason = ?state.mismatch_reason,
                        "clawTap cluster mismatch; solve blocked until tap matches PG clusterId+hash"
                    );
                }
                *handle.write().await = Some(state);
            }
            Ok(None) => {
                *handle.write().await = None;
            }
            Err(e) => {
                warn!(
                    target: "claw_gateway_orchestration",
                    component = "claw_tap_cluster",
                    error = %e,
                    "clawTap cluster refresh failed"
                );
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolveLlmRoute {
    pub mode: String,
    #[serde(rename = "clusterId")]
    pub cluster_id: String,
    #[serde(rename = "clusterHash")]
    pub cluster_hash: String,
    #[serde(rename = "clawTapBaseUrl", skip_serializing_if = "Option::is_none")]
    pub claw_tap_base_url: Option<String>,
    #[serde(rename = "upstreamBaseUrl")]
    pub upstream_base_url: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub fn active_llm_upstream(active: &ActiveLlmRuntime) -> Result<(String, String), String> {
    let upstream = normalize_upstream_base_url(&active.base_model_url)
        .ok_or_else(|| "active LLM baseModelUrl invalid".to_string())?;
    let model = normalize_model_name_for_upstream(&active.model_name, &active.base_model_url)
        .ok_or_else(|| "active LLM modelName invalid".to_string())?;
    Ok((upstream, model))
}

pub async fn resolve_solve_llm_route(
    db: &GatewaySessionDb,
    cluster_handle: &ClawTapClusterHandle,
    _llm_handle: &LlmRuntimeHandle,
    model_override: Option<&str>,
) -> Result<(SolveLlmRoute, std::collections::BTreeMap<String, String>), String> {
    let snapshot = snapshot_from_handle(cluster_handle).await;
    if snapshot.consistency == TapConsistency::ClusterMismatch {
        return Err(snapshot.reason.unwrap_or_else(|| {
            "clawTap cluster identity mismatch; fix tap or clusterId before solve".into()
        }));
    }
    if !is_ready(&snapshot) {
        return Err(
            "CLAW_CLUSTER_ID and clawTap must be configured and verified before solve".into(),
        );
    }
    let inner = cluster_handle
        .read()
        .await
        .clone()
        .ok_or_else(|| "clawTap cluster state unavailable".to_string())?;
    if inner.consistency != TapConsistency::Strict {
        return Err(inner
            .mismatch_reason
            .unwrap_or_else(|| "clawTap not in strict cluster mode".into()));
    }
    let active = gateway_global_settings::load_active_llm_runtime(db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no active LLM model configured in Admin".to_string())?;
    let (upstream, default_model) = active_llm_upstream(&active)?;
    let model = model_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(default_model);
    let api_key = active.api_key.trim();
    if api_key.is_empty() {
        return Err("active LLM apiKey missing".into());
    }
    let openai_base = inner.tap_base_url.clone();
    let route = SolveLlmRoute {
        mode: "clawTap".to_string(),
        cluster_id: inner.cluster_id.clone(),
        cluster_hash: inner.local_identity.cluster_hash.clone(),
        claw_tap_base_url: Some(inner.tap_base_url.clone()),
        upstream_base_url: upstream,
        model: model.clone(),
        reason: None,
    };
    let mut env = std::collections::BTreeMap::new();
    env.insert("OPENAI_BASE_URL".to_string(), openai_base.clone());
    env.insert("OPENAI_API_KEY".to_string(), api_key.to_string());
    env.insert("CLAW_DEFAULT_MODEL".to_string(), model);
    env.insert("INTERNAL_CLAUDE_TAP_HOST".to_string(), openai_base);
    Ok((route, env))
}
