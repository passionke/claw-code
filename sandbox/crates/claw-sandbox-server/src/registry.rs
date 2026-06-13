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

/// True when human `.env` pinned `CLAW_POOL_ADVERTISE_HOST` / `CLAW_POOL_ADVERTISE_IP`. Author: kejiqing
#[must_use]
pub fn advertise_host_pinned() -> bool {
    matches!(
        std::env::var("CLAW_POOL_ADVERTISE_HOST_PINNED")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("1" | "true" | "yes" | "TRUE" | "YES")
    )
}

pub fn resolve_advertise_host() -> String {
    if !advertise_host_pinned() {
        if let Some(ip) = detect_lan_ipv4() {
            return ip;
        }
    }
    for key in [
        "CLAW_POOL_ADVERTISE_HOST",
        "CLAW_POOL_ADVERTISE_IP",
        "CLAW_SANDBOX_ADVERTISE_HOST",
    ] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    if let Some(ip) = detect_lan_ipv4() {
        return ip;
    }
    std::env::var("HOSTNAME").unwrap_or_else(|_| "127.0.0.1".into())
}

/// Best-effort LAN IPv4 (UDP connect trick). Author: kejiqing
#[must_use]
pub fn detect_lan_ipv4() -> Option<String> {
    use std::net::{Ipv4Addr, UdpSocket};
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect((Ipv4Addr::new(1, 1, 1, 1), 80)).ok()?;
    match socket.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) if !v4.is_loopback() => Some(v4.to_string()),
        _ => None,
    }
}

pub fn resolve_gateway_base(advertise_ip: &str) -> String {
    if advertise_host_pinned() {
        for key in [
            "CLAW_POOL_GATEWAY_BASE",
            "CLAW_GATEWAY_BASE",
            "PLAYGROUND_PUBLIC_GATEWAY_BASE",
        ] {
            if let Ok(v) = std::env::var(key) {
                let t = v.trim().trim_end_matches('/');
                if !t.is_empty() {
                    return t.to_string();
                }
            }
        }
    }
    let port = std::env::var("GATEWAY_HOST_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(18088);
    format!("http://{advertise_ip}:{port}")
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
    let registration_time_ms = now;
    let slots_max_i32 = i32::try_from(slots_max).unwrap_or(i32::MAX);
    let slots_min_i32 = i32::try_from(slots_min).unwrap_or(0);
    let sse_port_i32 = i32::from(sse_port);
    let row = ClawPoolUpsert {
        pool_id: &pool_id,
        registration_time_ms,
        slots_max: slots_max_i32,
        slots_min: slots_min_i32,
        advertise_ip: &advertise_ip,
        sse_port: sse_port_i32,
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
    let mut last_advertise_ip = advertise_ip;
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            () = tokio::time::sleep(Duration::from_secs(60)) => {
                let ts = now_ms_for_registry();
                let advertise_ip = resolve_advertise_host();
                let gateway_base = resolve_gateway_base(&advertise_ip);
                if advertise_ip != last_advertise_ip {
                    info!(
                        target: "claw_sandbox",
                        component = "registry",
                        pool_id = %pool_id_hb,
                        old_advertise_ip = %last_advertise_ip,
                        new_advertise_ip = %advertise_ip,
                        gateway_base = %gateway_base,
                        "claw_pool advertise_ip changed; refreshing registry row"
                    );
                    last_advertise_ip = advertise_ip.clone();
                }
                let row = ClawPoolUpsert {
                    pool_id: &pool_id_hb,
                    registration_time_ms,
                    slots_max: slots_max_i32,
                    slots_min: slots_min_i32,
                    advertise_ip: &advertise_ip,
                    sse_port: sse_port_i32,
                    gateway_base: &gateway_base,
                    last_heartbeat_ms: ts,
                };
                if let Err(e) = db_hb.upsert_claw_pool(&row).await {
                    warn!(
                        target: "claw_sandbox",
                        component = "registry",
                        pool_id = %pool_id_hb,
                        error = %e,
                        "claw_pool heartbeat upsert failed"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{advertise_host_pinned, resolve_gateway_base};

    #[test]
    fn resolve_gateway_base_fallback_uses_advertise_ip_and_port() {
        assert!(!advertise_host_pinned());
        assert_eq!(resolve_gateway_base("10.1.2.3"), "http://10.1.2.3:18088");
    }
}
