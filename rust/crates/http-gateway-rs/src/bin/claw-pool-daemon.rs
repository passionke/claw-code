//! Host-side pool daemon: line-delimited JSON RPC over TCP (default) or Unix socket. Author: kejiqing

use std::path::PathBuf;

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
    let pool =
        match http_gateway_rs::pool::DockerPoolManager::try_from_env(podman, &pool_binding_root) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("claw-pool-daemon: {e}");
                std::process::exit(1);
            }
        };
    http_gateway_rs::pool::DockerPoolManager::schedule_warm(&pool);

    let tcp_bind = std::env::var("CLAW_POOL_DAEMON_TCP_BIND")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());

    if let Some(ref addr) = tcp_bind {
        tracing::info!(
            target: "claw_gateway_pool",
            component = "pool_daemon_main",
            phase = "start",
            tcp_bind = %addr,
            work_root = %work_root.display(),
            pool_bind_root = %pool_binding_root.display(),
            podman,
            "claw-pool-daemon (tcp)"
        );
        if let Err(e) = http_gateway_rs::pool::serve_pool_rpc_tcp(addr, pool).await {
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
        listen = %path.display(),
        work_root = %work_root.display(),
        pool_bind_root = %pool_binding_root.display(),
        podman,
        "claw-pool-daemon (unix)"
    );
    if let Err(e) = http_gateway_rs::pool::serve_pool_rpc(&path, pool).await {
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
