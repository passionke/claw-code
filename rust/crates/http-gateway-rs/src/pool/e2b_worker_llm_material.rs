//! Single entry for e2b worker LLM env (solve + interactive + OVS). Author: kejiqing

use std::collections::BTreeMap;

use crate::claw_tap_cluster_state::{active_llm_upstream, claw_repl_model_name, SolveLlmRoute};
use crate::cluster_identity::{gateway_cluster_id, gateway_database_url, local_cluster_identity};
use crate::gateway_global_settings;
use crate::gateway_project_llm::{
    load_active_project_llm_runtime, load_project_observe_proxy_base_url,
};
use crate::session_db::GatewaySessionDb;

use super::interactive_backend::{
    e2b_worker_llm_env, e2b_worker_solve_route, load_e2b_observe_proxy_base_url,
};

const E2B_SOLVE_PASSTHROUGH_ENV_KEYS: &[&str] = &[
    "CLAW_SSE_BURST_TRACE",
    "CLAW_SSE_BURST_LOG_FILE",
    "CLAW_SSE_DEBUG",
    "CLAW_SSE_LOG_FILE",
    "CLAW_SSE_DEBUG_PREVIEW_CHARS",
];

fn extend_env_from_gateway_process(
    mut env: BTreeMap<String, String>,
    passthrough_keys: &[&str],
) -> BTreeMap<String, String> {
    for key in passthrough_keys {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        env.insert((*key).to_string(), value);
    }
    env
}

fn log_sse_env_passthrough(proj_id: i64, env: &BTreeMap<String, String>) {
    let pairs: Vec<String> = E2B_SOLVE_PASSTHROUGH_ENV_KEYS
        .iter()
        .filter_map(|key| env.get(*key).map(|value| format!("{key}={value}")))
        .collect();
    tracing::info!(
        target: "claw_sse_env",
        proj_id = proj_id,
        count = pairs.len(),
        values = %pairs.join(" "),
        "e2b.solve_env_passthrough"
    );
}

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

/// e2b-only: project override LLM+observe when configured, else global active + global observe.
pub async fn prepare_e2b_worker_llm_material(
    session_db: &GatewaySessionDb,
    proj_id: i64,
    model_override: Option<&str>,
    options: PrepareE2bWorkerLlmOptions,
) -> Result<WorkerLlmMaterial, String> {
    let cluster_id = gateway_cluster_id()?;
    let db_url = gateway_database_url()?;
    let local = local_cluster_identity(&cluster_id, &db_url)?;

    let project_active = load_active_project_llm_runtime(session_db, proj_id)
        .await
        .map_err(|e| e.to_string())?;

    let (proxy_base, active, mode) = if let Some(active) = project_active {
        let proxy = load_project_observe_proxy_base_url(session_db, proj_id)
            .await?
            .ok_or_else(|| {
                format!(
                    "project {proj_id} has custom LLM but project observe proxyBaseUrl missing; \
                     Admin → 项目推理 reset observe"
                )
            })?;
        (proxy, active, "e2bProjectObserveTap")
    } else {
        let proxy = load_e2b_observe_proxy_base_url(session_db).await?;
        let active = gateway_global_settings::load_active_llm_runtime(session_db)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no active LLM model configured in Admin".to_string())?;
        (proxy, active, "e2bObserveTap")
    };

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
            mode: mode.to_string(),
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
    env = extend_env_from_gateway_process(env, E2B_SOLVE_PASSTHROUGH_ENV_KEYS);
    log_sse_env_passthrough(proj_id, &env);
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
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

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

    #[test]
    fn extend_env_from_gateway_process_forwards_sse_trace_and_debug_keys() {
        let _guard = env_lock();
        std::env::set_var("CLAW_SSE_BURST_TRACE", "1");
        std::env::set_var(
            "CLAW_SSE_BURST_LOG_FILE",
            "/claw_sessions/sse-burst-trace.ndjson",
        );
        std::env::set_var("CLAW_SSE_DEBUG", "1");
        std::env::set_var("CLAW_SSE_LOG_FILE", "/claw_sessions/sse-debug.log");
        std::env::set_var("CLAW_SSE_DEBUG_PREVIEW_CHARS", "2500");
        let out = extend_env_from_gateway_process(BTreeMap::new(), E2B_SOLVE_PASSTHROUGH_ENV_KEYS);
        assert_eq!(
            out.get("CLAW_SSE_BURST_TRACE").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            out.get("CLAW_SSE_BURST_LOG_FILE").map(String::as_str),
            Some("/claw_sessions/sse-burst-trace.ndjson")
        );
        assert_eq!(out.get("CLAW_SSE_DEBUG").map(String::as_str), Some("1"));
        assert_eq!(
            out.get("CLAW_SSE_LOG_FILE").map(String::as_str),
            Some("/claw_sessions/sse-debug.log")
        );
        assert_eq!(
            out.get("CLAW_SSE_DEBUG_PREVIEW_CHARS").map(String::as_str),
            Some("2500")
        );
        std::env::remove_var("CLAW_SSE_BURST_TRACE");
        std::env::remove_var("CLAW_SSE_BURST_LOG_FILE");
        std::env::remove_var("CLAW_SSE_DEBUG");
        std::env::remove_var("CLAW_SSE_LOG_FILE");
        std::env::remove_var("CLAW_SSE_DEBUG_PREVIEW_CHARS");
    }
}
