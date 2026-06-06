//! Host-side pool daemon: line-delimited JSON RPC + HTTP live report SSE. Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;

use http_gateway_rs::pool::{
    serve_pool_http, serve_pool_rpc, serve_pool_rpc_tcp, DockerPoolManager, LiveReportHub,
};
use http_gateway_rs::pool_registry;
use http_gateway_rs::pool_worker_runtime_sync;
use http_gateway_rs::session_db::GatewaySessionDb;
use tokio::sync::RwLock;
use tracing::warn;

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() {
    let work_root = PathBuf::from(
        std::env::var("CLAW_WORK_ROOT").unwrap_or_else(|_| "/tmp/claw-workspace".into()),
    );
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let solve_isolation = std::env::var("CLAW_SOLVE_ISOLATION")
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let podman = match solve_isolation.as_str() {
        "docker_pool" => false,
        "podman_pool" | "" => true,
        other => {
            eprintln!(
                "claw-pool-daemon: invalid CLAW_SOLVE_ISOLATION={other:?}; use podman_pool or docker_pool."
            );
            std::process::exit(1);
        }
    };

    let pool_binding_root = pool_host_bind_root(&work_root);
    let hub = Arc::new(LiveReportHub::default());

    let pool_id = pool_registry::resolve_pool_id();
    let registry_db = match GatewaySessionDb::open().await {
        Ok(db) => Some(Arc::new(db)),
        Err(e) => {
            warn!(
                target: "claw_gateway_pool",
                component = "pool_daemon_main",
                error = %e,
                "CLAW_GATEWAY_DATABASE_URL missing or invalid; claw_pool registry disabled"
            );
            None
        }
    };
    let registry = registry_db
        .as_ref()
        .map(|db| (pool_id.clone(), Arc::clone(db)));

    let pool =
        match DockerPoolManager::try_from_env(podman, &pool_binding_root, Some(hub), registry) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("claw-pool-daemon: {e}");
                std::process::exit(1);
            }
        };
    DockerPoolManager::schedule_warm(&pool);
    http_gateway_rs::live_report_audit::log_live_report_startup(
        "claw-pool-daemon",
        "pool_local_hub",
    );

    let http_bind = std::env::var("CLAW_POOL_HTTP_BIND")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "0.0.0.0:9944".to_string());
    let sse_port = pool_registry::port_from_bind(&http_bind);
    let advertise_ip = pool_registry::resolve_advertise_host();
    let gateway_base = pool_registry::resolve_gateway_base(&advertise_ip);

    if let Some(db) = registry_db {
        let llm_runtime = std::sync::Arc::new(RwLock::new(None));
        if pool_worker_runtime_sync::resolve_repo_root().is_some() {
            let db_poll = Arc::clone(&db);
            let llm_poll = std::sync::Arc::clone(&llm_runtime);
            tokio::task::spawn_blocking(move || {
                pool_worker_runtime_sync::pool_worker_runtime_poll_loop(db_poll, llm_poll);
            });
        } else {
            warn!(
                target: "claw_gateway_pool",
                component = "pool_daemon_main",
                "CLAW_REPO_ROOT unset; pool worker runtime DB poll disabled"
            );
        }
        let (slots_max, slots_min) = pool.slot_capacity();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(pool_registry::run_pool_registry(
            db,
            pool_id.clone(),
            advertise_ip.clone(),
            gateway_base.clone(),
            sse_port,
            slots_max,
            slots_min,
            shutdown_rx,
        ));
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                let _ = shutdown_tx.send(true);
            }
        });
    }

    let pool_http = Arc::clone(&pool);

    let tcp_bind = std::env::var("CLAW_POOL_DAEMON_TCP_BIND")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    let listen = std::env::var("CLAW_POOL_DAEMON_LISTEN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    if let Some(ref addr) = tcp_bind {
        let http_bind_spawn = http_bind.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_pool_http(
                &http_bind_spawn,
                Arc::clone(&pool_http),
                std::future::pending::<()>(),
            )
            .await
            {
                tracing::error!(
                    target: "claw_gateway_pool",
                    component = "pool_daemon_main",
                    error = %e,
                    "claw-pool-daemon http server exited"
                );
            }
        });
        tracing::info!(
            target: "claw_gateway_pool",
            component = "pool_daemon_main",
            phase = "start",
            pool_id = %pool_id,
            tcp_bind = %addr,
            http_bind = %http_bind,
            advertise_ip = %advertise_ip,
            sse_port,
            work_root = %work_root.display(),
            pool_bind_root = %pool_binding_root.display(),
            podman,
            "claw-pool-daemon (legacy tcp + http)"
        );
        if let Err(e) = serve_pool_rpc_tcp(addr, pool).await {
            eprintln!("claw-pool-daemon: {e}");
            std::process::exit(1);
        }
        return;
    }

    if let Some(ref path) = listen {
        let http_bind_spawn = http_bind.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_pool_http(
                &http_bind_spawn,
                Arc::clone(&pool_http),
                std::future::pending::<()>(),
            )
            .await
            {
                tracing::error!(
                    target: "claw_gateway_pool",
                    component = "pool_daemon_main",
                    error = %e,
                    "claw-pool-daemon http server exited"
                );
            }
        });
        let path = PathBuf::from(path);
        tracing::info!(
            target: "claw_gateway_pool",
            component = "pool_daemon_main",
            phase = "start",
            pool_id = %pool_id,
            listen = %path.display(),
            http_bind = %http_bind,
            advertise_ip = %advertise_ip,
            sse_port,
            work_root = %work_root.display(),
            pool_bind_root = %pool_binding_root.display(),
            podman,
            "claw-pool-daemon (legacy unix + http)"
        );
        if let Err(e) = serve_pool_rpc(&path, pool).await {
            eprintln!("claw-pool-daemon: {e}");
            std::process::exit(1);
        }
        return;
    }

    tracing::info!(
        target: "claw_gateway_pool",
        component = "pool_daemon_main",
        phase = "start",
        pool_id = %pool_id,
        http_bind = %http_bind,
        advertise_ip = %advertise_ip,
        sse_port,
        work_root = %work_root.display(),
        pool_bind_root = %pool_binding_root.display(),
        podman,
        "claw-pool-daemon http-only (RPC POST /v1/pool/rpc + live SSE)"
    );
    // HTTP is the main loop; SIGTERM/SIGINT only triggers graceful shutdown (no select race). kejiqing
    let shutdown = async {
        let reason = pool_daemon_shutdown_signal().await;
        tracing::info!(
            target: "claw_gateway_pool",
            component = "pool_daemon_main",
            reason = %reason,
            "claw-pool-daemon shutting down"
        );
    };
    if let Err(e) = serve_pool_http(&http_bind, pool_http, shutdown).await {
        tracing::error!(
            target: "claw_gateway_pool",
            component = "pool_daemon_main",
            error = %e,
            "claw-pool-daemon http server exited"
        );
        std::process::exit(1);
    }
}

async fn pool_daemon_shutdown_signal() -> String {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            tokio::select! {
                res = tokio::signal::ctrl_c() => {
                    if res.is_ok() {
                        return "SIGINT".to_string();
                    }
                    return "ctrl_c_error".to_string();
                }
                _ = sigterm.recv() => return "SIGTERM".to_string(),
            }
        }
    }
    if tokio::signal::ctrl_c().await.is_ok() {
        "SIGINT".to_string()
    } else {
        "ctrl_c_error".to_string()
    }
}

fn pool_host_bind_root(work_root: &std::path::Path) -> PathBuf {
    if let Ok(raw) = std::env::var("CLAW_POOL_WORK_ROOT_HOST") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.exists() {
                return p;
            }
            tracing::warn!(
                target: "claw_gateway_pool",
                component = "pool_daemon_main",
                phase = "pool_host_bind_root_fallback",
                configured = %trimmed,
                fallback = %work_root.display(),
                "CLAW_POOL_WORK_ROOT_HOST missing on host; using CLAW_WORK_ROOT"
            );
        }
    }
    work_root.to_path_buf()
}
