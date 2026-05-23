//! Host-side pool daemon: line-delimited JSON RPC + HTTP live report SSE. Author: kejiqing

use std::path::PathBuf;
use std::sync::Arc;

use http_gateway_rs::pool::{
    serve_pool_http, serve_pool_rpc, serve_pool_rpc_tcp, DockerPoolManager, LiveReportHub,
};
use http_gateway_rs::pool_registry;
use http_gateway_rs::session_db::GatewaySessionDb;
use tracing::warn;

#[tokio::main]
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

    if let Some(db) = registry_db {
        let (slots_max, slots_min) = pool.slot_capacity();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(pool_registry::run_pool_registry(
            db,
            pool_id.clone(),
            advertise_ip.clone(),
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
    let http_bind_spawn = http_bind.clone();
    tokio::spawn(async move {
        if let Err(e) = serve_pool_http(&http_bind_spawn, pool_http).await {
            tracing::error!(
                target: "claw_gateway_pool",
                component = "pool_daemon_main",
                error = %e,
                "claw-pool-daemon http server exited"
            );
        }
    });

    let tcp_bind = std::env::var("CLAW_POOL_DAEMON_TCP_BIND")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    if let Some(ref addr) = tcp_bind {
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
            "claw-pool-daemon (tcp + http)"
        );
        if let Err(e) = serve_pool_rpc_tcp(addr, pool).await {
            eprintln!("claw-pool-daemon: {e}");
            std::process::exit(1);
        }
        return;
    }

    let listen = std::env::var("CLAW_POOL_DAEMON_LISTEN")
        .unwrap_or_else(|_| "/tmp/claw-pool-daemon.sock".into());
    let path = PathBuf::from(listen);
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
        "claw-pool-daemon (unix + http)"
    );
    if let Err(e) = serve_pool_rpc(&path, pool).await {
        eprintln!("claw-pool-daemon: {e}");
        std::process::exit(1);
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
