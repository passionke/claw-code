//! `claw_pool` registration and heartbeat for multi-host routing. Author: kejiqing

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tracing::{info, warn};

use crate::session_db::{ClawPoolUpsert, GatewaySessionDb};

/// Parse trailing `:port` from bind strings like `0.0.0.0:9944`.
#[must_use]
pub fn port_from_bind(bind: &str) -> u16 {
    let bind = bind.trim();
    bind.rsplit(':')
        .next()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9944)
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

/// Host/IP clients use to reach this pool's SSE (and RPC) from other machines.
#[must_use]
pub fn resolve_advertise_host() -> String {
    if !advertise_host_pinned() {
        if let Some(ip) = detect_lan_ipv4() {
            return ip;
        }
    }
    for key in ["CLAW_POOL_ADVERTISE_HOST", "CLAW_POOL_ADVERTISE_IP"] {
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
    std::env::var("HOSTNAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Best-effort LAN IPv4 (UDP connect trick; no extra deps). Author: kejiqing
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

/// Browser-reachable gateway base URL reported with pool registration. Author: kejiqing
#[must_use]
pub fn resolve_gateway_base(advertise_ip: &str) -> String {
    if advertise_host_pinned() {
        for key in ["CLAW_POOL_GATEWAY_BASE", "PLAYGROUND_PUBLIC_GATEWAY_BASE"] {
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
        .and_then(|p| p.trim().parse::<u16>().ok())
        .unwrap_or(18088);
    format!("http://{advertise_ip}:{port}")
}

/// Stable pool identity per machine (`CLAW_POOL_ID` or `pool-{hostname}`). Author: kejiqing
#[must_use]
pub fn resolve_pool_id() -> String {
    if let Ok(v) = std::env::var("CLAW_POOL_ID") {
        let t = v.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    let host = resolve_advertise_host();
    let slug = sanitize_pool_id_segment(&host);
    if slug.is_empty() {
        return "pool-local".to_string();
    }
    format!("pool-{slug}")
}

fn relaxed_worker_allowed_from_env() -> bool {
    match std::env::var("CLAW_ALLOW_RELAXED_WORKER") {
        Ok(v) => {
            let t = v.trim();
            !matches!(t, "0" | "false" | "no" | "off" | "FALSE" | "NO" | "OFF")
        }
        Err(_) => true,
    }
}

/// Drop offline legacy claw_pool rows (pre-unified pool_id) after re-register. kejiqing
async fn prune_superseded_offline_pools(
    db: &GatewaySessionDb,
    pool_id: &str,
    advertise_ip: &str,
    now_ms: i64,
) {
    let Some(base) = pool_id.strip_suffix("-strict") else {
        return;
    };
    if base.is_empty() {
        return;
    }
    let mut legacy_ids = vec![base.to_string()];
    legacy_ids.push(format!("{base}-relaxed"));
    for legacy_id in legacy_ids {
        if legacy_id == pool_id {
            continue;
        }
        if legacy_id.ends_with("-relaxed") && relaxed_worker_allowed_from_env() {
            continue;
        }
        match db
            .delete_claw_pool_if_offline(&legacy_id, advertise_ip, now_ms)
            .await
        {
            Ok(true) => info!(
                target: "claw_gateway_pool",
                component = "pool_registry",
                pruned_pool_id = %legacy_id,
                advertise_ip = %advertise_ip,
                "pruned offline superseded claw_pool row"
            ),
            Ok(false) => {}
            Err(e) => warn!(
                target: "claw_gateway_pool",
                component = "pool_registry",
                pruned_pool_id = %legacy_id,
                error = %e,
                "claw_pool prune failed"
            ),
        }
    }
}

fn sanitize_pool_id_segment(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Register in `claw_pool` and run 60s heartbeat until `shutdown` fires.
pub async fn run_pool_registry(
    db: Arc<GatewaySessionDb>,
    pool_id: String,
    advertise_ip: String,
    gateway_base: String,
    sse_port: u16,
    slots_max: usize,
    slots_min: usize,
    mut shutdown: watch::Receiver<bool>,
) {
    let now = crate::session_db::now_ms_for_registry();
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
        Ok(()) => {
            info!(
                target: "claw_gateway_pool",
                component = "pool_registry",
                pool_id = %pool_id,
                advertise_ip = %advertise_ip,
                gateway_base = %gateway_base,
                sse_port,
                slots_max,
                slots_min,
                "claw_pool registered"
            );
            prune_superseded_offline_pools(&db, &pool_id, &advertise_ip, now).await;
        }
        Err(e) => {
            warn!(
                target: "claw_gateway_pool",
                component = "pool_registry",
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
                let ts = crate::session_db::now_ms_for_registry();
                let advertise_ip = resolve_advertise_host();
                let gateway_base = resolve_gateway_base(&advertise_ip);
                if advertise_ip != last_advertise_ip {
                    info!(
                        target: "claw_gateway_pool",
                        component = "pool_registry",
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
                        target: "claw_gateway_pool",
                        component = "pool_registry",
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
    use super::{detect_lan_ipv4, resolve_gateway_base, sanitize_pool_id_segment};

    #[test]
    fn sanitize_pool_id_replaces_spaces() {
        assert_eq!(sanitize_pool_id_segment("my host"), "my-host");
    }

    #[test]
    fn resolve_gateway_base_fallback_uses_advertise_ip_and_port() {
        assert_eq!(resolve_gateway_base("10.1.2.3"), "http://10.1.2.3:18088");
    }

    #[test]
    fn detect_lan_ipv4_returns_dotted_quad_or_none() {
        if let Some(ip) = detect_lan_ipv4() {
            assert!(ip.parse::<std::net::Ipv4Addr>().is_ok());
        }
    }
}
