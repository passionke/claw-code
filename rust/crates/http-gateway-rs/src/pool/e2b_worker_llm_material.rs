//! Single entry for e2b worker LLM env (solve + interactive + OVS). Author: kejiqing

use std::collections::BTreeMap;

use crate::claw_tap_cluster_state::{active_llm_upstream, claw_repl_model_name, SolveLlmRoute};
use crate::cluster_identity::{gateway_cluster_id, gateway_database_url, local_cluster_identity};
use crate::gateway_global_settings;
use crate::session_db::GatewaySessionDb;

use super::interactive_backend::{
    e2b_worker_llm_env, e2b_worker_solve_route, load_e2b_observe_proxy_base_url,
};

/// Prepared LLM route + worker env + claw `--model` for e2b exec paths.
#[derive(Debug, Clone)]
pub struct WorkerLlmMaterial {
    pub route: SolveLlmRoute,
    pub env: BTreeMap<String, String>,
    /// Wire model for solve metadata; REPL-prefixed when `for_repl`.
    pub model: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PrepareE2bWorkerLlmOptions {
    /// Prefix bare upstream ids with `openai/` for interactive REPL / OVS.
    pub for_repl: bool,
}

/// e2b-only: PG active LLM + observe singleton proxy → worker-safe env (placeholder key).
pub async fn prepare_e2b_worker_llm_material(
    session_db: &GatewaySessionDb,
    model_override: Option<&str>,
    options: PrepareE2bWorkerLlmOptions,
) -> Result<WorkerLlmMaterial, String> {
    let cluster_id = gateway_cluster_id()?;
    let db_url = gateway_database_url()?;
    let local = local_cluster_identity(&cluster_id, &db_url)?;
    let proxy_base = load_e2b_observe_proxy_base_url(session_db).await?;
    let active = gateway_global_settings::load_active_llm_runtime(session_db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no active LLM model configured in Admin".to_string())?;
    let (upstream, default_model) = active_llm_upstream(&active)?;
    let wire_model = model_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or(default_model);
    let claw_model = if options.for_repl {
        claw_repl_model_name(&wire_model)
    } else {
        wire_model.clone()
    };
    let route = e2b_worker_solve_route(
        SolveLlmRoute {
            mode: "e2bObserveTap".to_string(),
            cluster_id: cluster_id.clone(),
            cluster_hash: local.cluster_hash.clone(),
            claw_tap_base_url: Some(proxy_base.clone()),
            upstream_base_url: upstream,
            model: wire_model.clone(),
            reason: None,
        },
        &proxy_base,
    );
    let mut env = BTreeMap::new();
    env.insert("CLAW_DEFAULT_MODEL".to_string(), claw_model.clone());
    let env = e2b_worker_llm_env(env, &proxy_base);
    Ok(WorkerLlmMaterial {
        route,
        env,
        model: claw_model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::interactive_backend::E2B_WORKER_TAP_PLACEHOLDER_API_KEY;

    #[test]
    fn prepare_options_default_not_repl() {
        let opts = PrepareE2bWorkerLlmOptions::default();
        assert!(!opts.for_repl);
    }

    #[test]
    fn worker_llm_material_env_uses_observe_placeholder_key() {
        let mut env = std::collections::BTreeMap::new();
        env.insert(
            "CLAW_DEFAULT_MODEL".to_string(),
            "openai/mimo-v2.5".to_string(),
        );
        let proxy = "http://8080-sbx_abc.supone.top";
        let out = e2b_worker_llm_env(env, proxy);
        assert_eq!(out.get("OPENAI_BASE_URL").map(String::as_str), Some(proxy));
        assert_eq!(
            out.get("OPENAI_API_KEY").map(String::as_str),
            Some(E2B_WORKER_TAP_PLACEHOLDER_API_KEY)
        );
        assert_eq!(
            out.get("CLAW_DEFAULT_MODEL").map(String::as_str),
            Some("openai/mimo-v2.5")
        );
    }
}
