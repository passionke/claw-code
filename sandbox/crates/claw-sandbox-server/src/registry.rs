//! `claw_pool` registration and heartbeat. Author: kejiqing

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tracing::{info, warn};

use crate::registry_db::{now_ms_for_registry, ClawPoolUpsert, PoolRegistryDb};

pub fn port_from_bind(bind: &str) -> u16 {
    bind.rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9944)
}

pub fn resolve_pool_id() -> String {
    std::env::var("CLAW_POOL_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            let host = std::env::var("CLAW_POOL_ADVERTISE_HOST")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "127.0.0.1".into());
            format!("pool-{host}")
        })
}

pub fn resolve_advertise_host() -> String {
    std::env::var("CLAW_POOL_ADVERTISE_HOST")
        .or_else(|_| std::env::var("CLAW_SANDBOX_ADVERTISE_HOST"))
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| std::env::var("HOSTNAME").unwrap_or_else(|_| "127.0.0.1".into()))
}

pub fn resolve_gateway_base(advertise_ip: &str) -> String {
    std::env::var("CLAW_GATEWAY_BASE")
        .ok()
        .map(|v| v.trim().trim_end_matches('/').to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            let port = std::env::var("GATEWAY_HOST_PORT")
                .ok()
                .and_then(|v| v.parse::<u16>().ok())
                .unwrap_or(18088);
            format!("http://{advertise_ip}:{port}")
        })
}

/// Register in `claw_pool` and run 60s heartbeat until `shutdown` fires.
pub async fn run_pool_registry(
    db: Arc<PoolRegistryDb>,
    pool_id: String,
    advertise_ip: String,
    gateway_base: String,
    sse_port: u16,
    slots_max: usize,
    slots_min: usize,
    mut shutdown: watch::Receiver<bool>,
) {
    let now = now_ms_for_registry();
    let row = ClawPoolUpsert {
        pool_id: &pool_id,
        registration_time_ms: now,
        slots_max: i32::try_from(slots_max).unwrap_or(i32::MAX),
        slots_min: i32::try_from(slots_min).unwrap_or(0),
        advertise_ip: &advertise_ip,
        sse_port: i32::from(sse_port),
        gateway_base: &gateway_base,
        last_heartbeat_ms: now,
    };
    match db.upsert_claw_pool(&row).await {
        Ok(()) => info!(
            target: "claw_sandbox",
            component = "registry",
            pool_id = %pool_id,
            advertise_ip = %advertise_ip,
            gateway_base = %gateway_base,
            sse_port,
            slots_max,
            slots_min,
            "claw_pool registered"
        ),
        Err(e) => {
            warn!(
                target: "claw_sandbox",
                component = "registry",
                error = %e,
                "claw_pool register failed; heartbeats disabled"
            );
            return;
        }
    }

    let pool_id_hb = pool_id.clone();
    let db_hb = Arc::clone(&db);
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            () = tokio::time::sleep(Duration::from_secs(60)) => {
                let ts = now_ms_for_registry();
                if let Err(e) = db_hb.touch_claw_pool_heartbeat(&pool_id_hb, ts).await {
                    warn!(
                        target: "claw_sandbox",
                        component = "registry",
                        pool_id = %pool_id_hb,
                        error = %e,
                        "claw_pool heartbeat failed"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_gateway_base;

    #[test]
    fn resolve_gateway_base_fallback_uses_advertise_ip_and_port() {
        assert_eq!(resolve_gateway_base("10.1.2.3"), "http://10.1.2.3:18088");
    }
}
