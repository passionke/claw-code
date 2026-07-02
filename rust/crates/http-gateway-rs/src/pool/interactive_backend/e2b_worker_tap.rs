//! e2b worker LLM routing via **observe singleton** claude-tap proxy (`8080-{sbx}.{domain}`).
//! Workers never start local tap, hold real API keys, or write LLM traces.
//! Author: kejiqing

use std::collections::BTreeMap;

use crate::claw_tap_cluster_state::{active_llm_upstream, SolveLlmRoute};
use crate::cluster_identity::{gateway_cluster_id, gateway_database_url, local_cluster_identity};
use crate::gateway_claw_tap_settings::claw_tap_proxy_base_url;
use crate::gateway_global_settings;
use crate::session_db::GatewaySessionDb;

/// Placeholder key sent worker→observe; observe injects real key from PG cluster state.
pub const E2B_WORKER_TAP_PLACEHOLDER_API_KEY: &str = "claw-tap-cluster";

/// Read observe singleton proxy URL from persisted clawTap settings (no localhost fallback).
pub async fn load_e2b_observe_proxy_base_url(
    session_db: &GatewaySessionDb,
) -> Result<String, String> {
    let (settings, _, _) = gateway_global_settings::get_gateway_global_settings(session_db)
        .await
        .map_err(|e| e.to_string())?;
    let tap = settings.claw_tap;
    if tap.updated_at_ms <= 0 {
        return Err(
            "e2b observe tap not configured: run gateway.sh observe-tap-up (clawTap settings empty)"
                .into(),
        );
    }
    tap.proxy_base_url
        .filter(|s| !s.trim().is_empty())
        .or_else(|| claw_tap_proxy_base_url(&tap.host, tap.proxy_port))
        .ok_or_else(|| "e2b observe tap proxyBaseUrl missing: run gateway.sh observe-tap-up".into())
}

/// Worker LLM env points at observe proxy; placeholder key only (no real credentials).
#[must_use]
pub fn e2b_worker_llm_env(
    mut env: BTreeMap<String, String>,
    proxy_base_url: &str,
) -> BTreeMap<String, String> {
    let proxy = proxy_base_url.trim().trim_end_matches('/').to_string();
    env.insert("OPENAI_BASE_URL".to_string(), proxy.clone());
    env.insert("INTERNAL_CLAUDE_TAP_HOST".to_string(), proxy);
    env.insert(
        "OPENAI_API_KEY".to_string(),
        E2B_WORKER_TAP_PLACEHOLDER_API_KEY.to_string(),
    );
    env
}

/// Async wrapper: load observe proxy from DB then apply worker-safe env.
pub async fn apply_e2b_observe_worker_llm_env(
    session_db: &GatewaySessionDb,
    env: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, String> {
    let proxy = load_e2b_observe_proxy_base_url(session_db).await?;
    Ok(e2b_worker_llm_env(env, &proxy))
}

/// Solve path: rewrite route metadata for observe singleton tap proxy.
#[must_use]
pub fn e2b_worker_solve_route(mut route: SolveLlmRoute, proxy_base_url: &str) -> SolveLlmRoute {
    route.claw_tap_base_url = Some(proxy_base_url.trim().trim_end_matches('/').to_string());
    route
}

/// e2b solve: observe singleton proxy; worker env has placeholder key only.
pub async fn resolve_e2b_worker_solve_llm_route(
    session_db: &GatewaySessionDb,
    model_override: Option<&str>,
) -> Result<(SolveLlmRoute, BTreeMap<String, String>), String> {
    let cluster_id = gateway_cluster_id()?;
    let db_url = gateway_database_url()?;
    let local = local_cluster_identity(&cluster_id, &db_url)?;
    let proxy_base = load_e2b_observe_proxy_base_url(session_db).await?;
    let active = gateway_global_settings::load_active_llm_runtime(session_db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no active LLM model configured in Admin".to_string())?;
    let (upstream, default_model) = active_llm_upstream(&active)?;
    let model = model_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(default_model);
    let route = e2b_worker_solve_route(
        SolveLlmRoute {
            mode: "e2bObserveTap".to_string(),
            cluster_id: cluster_id.clone(),
            cluster_hash: local.cluster_hash.clone(),
            claw_tap_base_url: Some(proxy_base.clone()),
            upstream_base_url: upstream,
            model: model.clone(),
            reason: None,
        },
        &proxy_base,
    );
    let mut env = BTreeMap::new();
    env.insert("CLAW_DEFAULT_MODEL".to_string(), model);
    Ok((route, e2b_worker_llm_env(env, &proxy_base)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e2b_worker_llm_env_points_at_observe_proxy() {
        let mut env = BTreeMap::new();
        env.insert(
            "OPENAI_BASE_URL".to_string(),
            "http://claw-claude-tap:8080".to_string(),
        );
        env.insert("OPENAI_API_KEY".to_string(), "sk-real-secret".to_string());
        let proxy = "http://8080-sbx_abc.supone.top";
        let out = e2b_worker_llm_env(env, proxy);
        assert_eq!(out.get("OPENAI_BASE_URL").map(String::as_str), Some(proxy));
        assert_eq!(
            out.get("OPENAI_API_KEY").map(String::as_str),
            Some(E2B_WORKER_TAP_PLACEHOLDER_API_KEY)
        );
    }
}
