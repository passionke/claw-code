//! Gateway process bootstrap. Author: kejiqing
#![allow(
    dead_code,
    clippy::too_many_lines,
    clippy::await_holding_lock,
    clippy::uninlined_format_args,
    clippy::cast_possible_truncation,
    clippy::manual_let_else
)]
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{
    claw_tap_cluster_state, gateway_cluster_bootstrap, gateway_llm_config_sync, gateway_logging,
    pool, session_db, session_terminal_api,
};
use gateway_solve_turn::ReportPolishDeepseek;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::app_state::{AppState, GatewayConfig};
use crate::routes::app::{
    gateway_env_enabled, now_ms, pool_host_bind_root, project_config_poll_loop,
    run_startup_project_config_apply, validate_projects_git_at_startup,
};

pub async fn run() {
    let work_root = PathBuf::from(
        std::env::var("CLAW_WORK_ROOT").unwrap_or_else(|_| "/tmp/claw-workspace".to_string()),
    );
    gateway_logging::init(&work_root);
    if std::env::var("OTEL_SERVICE_NAME")
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        std::env::set_var("OTEL_SERVICE_NAME", "claw-gateway-rs");
    }
    telemetry::init_otel_from_env();
    let file_log = gateway_logging::resolved_file_log_dir(&work_root);
    info!(
        target: "claw_gateway_orchestration",
        component = "startup",
        phase = "process_boot",
        work_root = %work_root.display(),
        solve_backend = "e2b",
        file_log_dir = file_log.as_ref().map(|p| p.display().to_string()),
        file_log_enabled = file_log.is_some(),
        stdout_json_forced_for_file_sink = file_log.is_some(),
        http_addr = %std::env::var("CLAW_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
        "http-gateway-rs tracing ready; when file_log_enabled, stdout is JSON too (same subscriber layers)"
    );
    let pool_binding_root = pool_host_bind_root(&work_root);
    info!(
        target: "claw_gateway_orchestration",
        component = "startup",
        phase = "pool_host_paths",
        work_root = %work_root.display(),
        pool_host_bind_root = %pool_binding_root.display(),
        "container pool uses pool_host_bind_root on the runtime host for worker -v mounts"
    );
    let pool_rpc_host_work_root = std::env::var("CLAW_POOL_RPC_HOST_WORK_ROOT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);

    // e2b cloud sandbox pool (local podman claw-sandbox removed). Author: kejiqing
    let live_report_hub = Arc::new(pool::LiveReportHub::default());
    let e2b_client = claw_e2b_sandbox_client::E2bSandboxConfig::from_env()
        .map(|cfg| Arc::new(claw_e2b_sandbox_client::E2bSandboxClient::new(cfg)));
    if let Some(ref fc) = e2b_client {
        if let Err(e) = fc.refresh_e2b_platform_nas().await {
            tracing::warn!(
                target: "claw_e2b_sandbox",
                error = %e,
                "startup e2b platform health fetch failed"
            );
        }
    }
    // e2b: NAS layout is claw-nas-api only (no gateway local mount fallback).
    if e2b_client.is_some() && !pool::E2bNasApiSingleton::enabled_from_env() {
        eprintln!("http-gateway-rs: CLAW_E2B_NAS_API must not be disabled in e2b mode");
        std::process::exit(1);
    }
    let nas_api = Arc::new(pool::E2bNasApiSingleton::new());
    let nas_layout = pool::NasLayoutBackend::new(Arc::clone(&nas_api));
    let pool_clients = pool::PoolClients::from_env(
        Arc::clone(&live_report_hub),
        work_root.clone(),
        e2b_client.clone(),
        pool_rpc_host_work_root.clone(),
        nas_layout,
    );
    let co_located_pool_id = Some(pool_clients.pool_id().to_string());
    tracing::info!(
        target: "claw_live_report",
        component = "gateway_startup",
        contract = crate::live_report_audit::LIVE_REPORT_CONTRACT,
        pool_id = %pool_clients.pool_id(),
        co_located_pool_id = ?co_located_pool_id,
        "live_report.gateway — terminal snapshot from DB; running live SSE from gateway LiveReportHub (e2b worker stdout relay)"
    );

    let projects_git_url = std::env::var("CLAW_PROJECTS_GIT_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_default();
    let projects_git_branch = std::env::var("CLAW_PROJECTS_GIT_BRANCH")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "main".to_string());
    let projects_git_author = std::env::var("CLAW_PROJECTS_GIT_AUTHOR")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "claw-gateway <noreply@claw.local>".to_string());
    let projects_git_token = std::env::var("CLAW_PROJECTS_GIT_TOKEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if !projects_git_url.is_empty() {
        validate_projects_git_at_startup(&projects_git_url, projects_git_token.as_deref());
    }

    let report_polish_deepseek = {
        let raw = std::env::var("REPORT_LLM_PROVIDER")
            .ok()
            .map(|v| v.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        match raw.as_deref() {
            None | Some("") => None,
            Some("deepseek") => {
                let api_key = std::env::var("DEEPSEEK_API_KEY")
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|s| !s.is_empty());
                if let Some(api_key) = api_key {
                    let model = std::env::var("REPORT_DEEPSEEK_MODEL")
                        .ok()
                        .map(|v| v.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "deepseek-v4-pro".to_string());
                    info!(
                        target: "claw_gateway_orchestration",
                        component = "startup",
                        phase = "report_llm",
                        provider = "deepseek",
                        model = %model,
                        "biz_advice_report polish routes to DeepSeek official API (DEEPSEEK_BASE_URL or default)"
                    );
                    Some(ReportPolishDeepseek { api_key, model })
                } else {
                    warn!(
                        target: "claw_gateway_orchestration",
                        component = "startup",
                        phase = "report_llm",
                        "REPORT_LLM_PROVIDER=deepseek but DEEPSEEK_API_KEY is empty; using default report LLM routing"
                    );
                    None
                }
            }
            Some(other) => {
                warn!(
                    target: "claw_gateway_orchestration",
                    component = "startup",
                    phase = "report_llm",
                    provider = %other,
                    "unknown REPORT_LLM_PROVIDER; expected unset or deepseek; using default report LLM routing"
                );
                None
            }
        }
    };

    let cfg = GatewayConfig {
        claw_bin: std::env::var("CLAW_BIN").unwrap_or_else(|_| "claw".to_string()),
        work_root,
        pool_rpc_host_work_root,
        co_located_pool_id,
        ds_registry_path: std::env::var("CLAW_DS_REGISTRY").map_or_else(
            |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("datasources.example.yaml"),
            PathBuf::from,
        ),
        default_timeout_seconds: std::env::var("CLAW_TIMEOUT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(120),
        default_max_iterations: std::env::var("CLAW_MAX_ITERATIONS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(64),
        default_http_mcp_name: std::env::var("CLAW_DEFAULT_HTTP_MCP_NAME")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        default_http_mcp_url: std::env::var("CLAW_DEFAULT_HTTP_MCP_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        default_http_mcp_transport: std::env::var("CLAW_DEFAULT_HTTP_MCP_TRANSPORT")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| v == "http" || v == "sse")
            .unwrap_or_else(|| "http".to_string()),
        projects_git_url,
        projects_git_branch,
        projects_git_author,
        projects_git_token,
        projects_git_proj_home_poll_interval_secs: std::env::var(
            "CLAW_PROJECT_CONFIG_POLL_INTERVAL_SECS",
        )
        .or_else(|_| std::env::var("CLAW_PROJECTS_GIT_DS_HOME_POLL_INTERVAL_SECS"))
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0),
        gateway_llm_config_poll_interval_secs: std::env::var(
            "CLAW_GATEWAY_LLM_CONFIG_POLL_INTERVAL_SECS",
        )
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .or(Some(30))
        .filter(|&s| s > 0),
        report_polish_deepseek,
        live_biz_report_spill_enabled: gateway_env_enabled("CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL"),
    };
    if cfg.live_biz_report_spill_enabled {
        info!(
            target: "claw_live_report",
            component = "startup",
            phase = "report_mode",
            mode = "legacy_spill_polish",
            "CLAW_GATEWAY_LIVE_BIZ_REPORT_SPILL=1 — hasReport on succeeded only; biz_advice_report uses LLM polish (no pool live SSE)"
        );
    }
    let session_db = session_db::GatewaySessionDb::open()
        .await
        .unwrap_or_else(|e| {
            eprintln!(
                "http-gateway-rs: failed to connect gateway PostgreSQL (CLAW_GATEWAY_DATABASE_URL): {e}"
            );
            std::process::exit(1);
        });
    match session_db
        .reconcile_interrupted_turns_on_startup(now_ms())
        .await
    {
        Ok(n) if n > 0 => {
            info!(
                target: "claw_gateway_orchestration",
                component = "startup",
                phase = "session_db_reconcile",
                reconciled_turn_rows = n,
                "marked in-flight gateway_turns as failed after gateway restart"
            );
        }
        Ok(_) => {}
        Err(e) => warn!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "session_db_reconcile",
            error = %e,
            "reconcile_interrupted_turns_on_startup failed"
        ),
    }
    let session_db = Arc::new(session_db);
    pool_clients.bind_session_db(Arc::clone(&session_db)).await;
    nas_api.bind_session_db(Arc::clone(&session_db)).await;
    let gateway_identity = Arc::new(
        crate::gateway_endpoint::resolve_gateway_endpoint_identity().unwrap_or_else(|e| {
            eprintln!("http-gateway-rs: CLAW_GATEWAY_ID/BASE resolve failed: {e}");
            std::process::exit(1);
        }),
    );
    if let Err(e) = crate::gateway_endpoint::register_and_spawn_heartbeat(
        Arc::clone(&session_db),
        (*gateway_identity).clone(),
    )
    .await
    {
        tracing::warn!(
            target: "claw_gateway_endpoint",
            error = %e,
            "gateway_endpoint register failed (best-effort)"
        );
    }

    let llm_runtime: gateway_llm_config_sync::LlmRuntimeHandle =
        Arc::new(tokio::sync::RwLock::new(None));
    let bootstrap_mode = match crate::gateway_cluster_bootstrap::cluster_needs_bootstrap(
        session_db.as_ref(),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!(
                target: "claw_gateway_bootstrap",
                error = %e,
                "bootstrap status check failed; assuming normal startup"
            );
            false
        }
    };

    if bootstrap_mode {
        info!(
            target: "claw_gateway_bootstrap",
            "cluster needs bootstrap — deferring strict e2b singleton ensure"
        );
        if let Ok(resp) =
            crate::gateway_cluster_bootstrap::apply_llm_from_env(session_db.as_ref(), &llm_runtime)
                .await
        {
            if resp.applied {
                info!(
                    target: "claw_gateway_bootstrap",
                    model = ?resp.model_name,
                    "startup: active LLM applied from env"
                );
            }
        }
    } else if let Err(e) = pool_clients
        .ensure_e2b_singletons_on_startup_strict(session_db.as_ref())
        .await
    {
        eprintln!("http-gateway-rs: e2b core singleton ensure failed (nas-api / observe): {e}");
        std::process::exit(1);
    }
    if let Err(e) = pool_clients.reconcile_project_workers_on_startup().await {
        tracing::warn!(
            target: "claw_e2b_proj_worker",
            error = %e,
            "startup project worker reconcile failed (best-effort)"
        );
    }
    info!(
        target: "claw_gateway_orchestration",
        component = "startup",
        phase = "session_db",
        gateway_database_url = %session_db.database_url_redacted(),
        "gateway session PostgreSQL ready (CLAW_GATEWAY_DATABASE_URL)"
    );
    let state = AppState {
        tasks: Arc::new(Mutex::new(HashMap::new())),
        injected_mcp: Arc::new(Mutex::new(HashMap::new())),
        proj_locks: Arc::new(Mutex::new(HashMap::new())),
        session_solve_locks: Arc::new(Mutex::new(HashMap::new())),
        session_db,
        cfg: Arc::new(cfg),
        docker_slots: Arc::new(Mutex::new(HashMap::new())),
        pool_clients,
        live_report_hub,
        projects_git_mirror_lock: Arc::new(Mutex::new(())),
        llm_runtime,
        claw_tap_cluster: Arc::new(tokio::sync::RwLock::new(None)),
        terminal_registry: session_terminal_api::TerminalSessionRegistry::new(),
        nas_api,
        gateway_identity: Arc::clone(&gateway_identity),
    };

    run_startup_project_config_apply(&state).await;
    gateway_llm_config_sync::run_startup_llm_config_sync(&state.session_db, &state.llm_runtime)
        .await;
    if let Ok(Some(cluster)) = claw_tap_cluster_state::refresh_claw_tap_cluster_state(
        &state.session_db,
        &state.llm_runtime,
    )
    .await
    {
        *state.claw_tap_cluster.write().await = Some(cluster);
    }

    if bootstrap_mode {
        gateway_cluster_bootstrap::spawn_bootstrap_reconcile_loop(
            Arc::clone(&state.session_db),
            state.pool_clients.clone(),
            state.llm_runtime.clone(),
            state.claw_tap_cluster.clone(),
        );
        info!(
            target: "claw_gateway_bootstrap",
            component = "startup",
            phase = "bootstrap_reconcile",
            "background cluster bootstrap reconcile enabled"
        );
    } else {
        let poll_db = state.session_db.clone();
        let poll_pool = state.pool_clients.clone();
        poll_pool.spawn_singleton_health_reconcile_loop(poll_db);
    }

    crate::master_scheduler::spawn_master_scheduler(state.clone());
    info!(
        target: "claw_master_scheduler",
        component = "startup",
        phase = "master_scheduler",
        "master observer scheduler ticker enabled"
    );

    {
        let poll_db = state.session_db.clone();
        let poll_llm = state.llm_runtime.clone();
        let poll_cluster = state.claw_tap_cluster.clone();
        tokio::spawn(async move {
            claw_tap_cluster_state::cluster_poll_loop(poll_db, poll_llm, poll_cluster).await;
        });
    }

    if let Some(secs) = state.cfg.gateway_llm_config_poll_interval_secs {
        let poll_db = state.session_db.clone();
        let poll_handle = state.llm_runtime.clone();
        tokio::spawn(async move {
            gateway_llm_config_sync::llm_config_poll_loop(poll_db, poll_handle, secs).await;
        });
        info!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "llm_config_poll",
            interval_secs = secs,
            "background LLM config sync poll enabled"
        );
    }

    if let Some(secs) = state.cfg.projects_git_proj_home_poll_interval_secs {
        let poller_state = state.clone();
        tokio::spawn(async move { project_config_poll_loop(poller_state, secs).await });
        info!(
            target: "claw_gateway_orchestration",
            component = "startup",
            phase = "project_config_poll",
            interval_secs = secs,
            "background project_config materialize poll enabled"
        );
    }

    let app = crate::routes::build_router(state.clone());

    let addr = std::env::var("CLAW_HTTP_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    info!("http gateway rs listening on {}", addr);
    let pool_clients_shutdown = state.pool_clients.clone();
    let shutdown = async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
                tokio::select! {
                    res = tokio::signal::ctrl_c() => {
                        if res.is_ok() {
                            info!(phase = "shutdown", "http gateway received SIGINT");
                        }
                    }
                    _ = sigterm.recv() => {
                        info!(phase = "shutdown", "http gateway received SIGTERM");
                    }
                }
            } else if tokio::signal::ctrl_c().await.is_ok() {
                info!(phase = "shutdown", "http gateway received SIGINT");
            }
        }
        #[cfg(not(unix))]
        if tokio::signal::ctrl_c().await.is_ok() {
            info!(phase = "shutdown", "http gateway received SIGINT");
        }
        pool_clients_shutdown.shutdown_e2b_sandboxes().await;
        telemetry::shutdown_otel();
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .expect("start axum");
}
