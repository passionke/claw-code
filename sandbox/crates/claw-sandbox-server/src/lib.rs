//! claw-sandbox server library. Author: kejiqing

pub mod pool;
pub mod registry;
pub mod registry_db;
pub mod runtime;

pub use claw_sandbox_protocol::{PoolRpcReq, PoolRpcResp, SandboxRpcReq, SandboxRpcResp};
pub use pool::{
    dispatch_pool_rpc, dispatch_sandbox_rpc, merge_stdout_hooks, serve_pool_http, serve_pool_rpc,
    serve_pool_rpc_tcp, DockerPoolManager, LiveReportHub,
};
pub use registry::{
    port_from_bind, resolve_advertise_host, resolve_gateway_base, resolve_pool_id,
    run_pool_registry,
};

use std::path::PathBuf;
use std::sync::Arc;

use tracing::{info, warn};

const SERVICE: &str = "claw-sandbox";

/// Run the sandbox HTTP server until shutdown. Author: kejiqing
pub async fn run() -> Result<(), String> {
    if std::env::var("OTEL_SERVICE_NAME")
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        std::env::set_var("OTEL_SERVICE_NAME", "claw-pool-daemon");
    }
    telemetry::init_otel_from_env();

    let work_root = PathBuf::from(
        std::env::var("CLAW_WORK_ROOT").unwrap_or_else(|_| "/tmp/claw-workspace".into()),
    );
    let solve_isolation = std::env::var("CLAW_SOLVE_ISOLATION")
        .map(|v| v.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let podman = match solve_isolation.as_str() {
        "docker_pool" => false,
        "podman_pool" | "" => true,
        other => {
            return Err(format!(
                "invalid CLAW_SOLVE_ISOLATION={other:?}; use podman_pool or docker_pool"
            ));
        }
    };

    let pool_binding_root = pool_host_bind_root(&work_root);
    let pool_id = resolve_pool_id();

    let registry_db = match registry_db::PoolRegistryDb::open().await {
        Ok(db) => Some(Arc::new(db)),
        Err(e) => {
            warn!(
                target: "claw_sandbox",
                component = "sandbox_main",
                error = %e,
                "claw_pool registry disabled"
            );
            None
        }
    };

    let pool =
        DockerPoolManager::try_from_env(podman, &pool_binding_root, None, Some(pool_id.clone()))
            .map_err(|e| e.to_string())?;
    DockerPoolManager::schedule_warm(&pool);

    let http_bind = std::env::var("CLAW_POOL_HTTP_BIND")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "0.0.0.0:9944".to_string());
    let sse_port = port_from_bind(&http_bind);
    let advertise_ip = resolve_advertise_host();
    let gateway_base = resolve_gateway_base(&advertise_ip);

    if let Some(db) = registry_db {
        let (slots_max, slots_min) = pool.slot_capacity();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        tokio::spawn(run_pool_registry(
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

    info!(
        target: "claw_sandbox",
        component = "sandbox_main",
        phase = "start",
        pool_id = %pool_id,
        http_bind = %http_bind,
        advertise_ip = %advertise_ip,
        work_root = %work_root.display(),
        podman,
        "{SERVICE} POST /v1/sandbox/rpc"
    );

    let shutdown = async {
        let reason = sandbox_shutdown_signal().await;
        telemetry::shutdown_otel();
        info!(
            target: "claw_sandbox",
            component = "sandbox_main",
            reason = %reason,
            "{SERVICE} shutting down"
        );
    };
    serve_pool_http(&http_bind, pool, shutdown).await
}

async fn sandbox_shutdown_signal() -> String {
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
        }
    }
    work_root.to_path_buf()
}
