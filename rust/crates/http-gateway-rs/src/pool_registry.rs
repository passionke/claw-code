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

/// Host/IP clients use to reach this pool's SSE (and RPC) from other machines.
#[must_use]
pub fn resolve_advertise_host() -> String {
    for key in ["CLAW_POOL_ADVERTISE_HOST", "CLAW_POOL_ADVERTISE_IP"] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    std::env::var("HOSTNAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Browser-reachable gateway base URL reported with pool registration. Author: kejiqing
#[must_use]
pub fn resolve_gateway_base(advertise_ip: &str) -> String {
    for key in ["CLAW_POOL_GATEWAY_BASE", "PLAYGROUND_PUBLIC_GATEWAY_BASE"] {
        if let Ok(v) = std::env::var(key) {
            let t = v.trim().trim_end_matches('/');
            if !t.is_empty() {
                return t.to_string();
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

/// Drop offline pre-dual-pool rows (and disabled relaxed) after strict re-registers. kejiqing
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
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            () = tokio::time::sleep(Duration::from_secs(60)) => {
                let ts = crate::session_db::now_ms_for_registry();
                if let Err(e) = db_hb.touch_claw_pool_heartbeat(&pool_id_hb, ts).await {
                    warn!(
                        target: "claw_gateway_pool",
                        component = "pool_registry",
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
    use super::{resolve_gateway_base, sanitize_pool_id_segment};

    #[test]
    fn sanitize_pool_id_replaces_spaces() {
        assert_eq!(sanitize_pool_id_segment("my host"), "my-host");
    }

    #[test]
    fn resolve_gateway_base_fallback_uses_advertise_ip_and_port() {
        assert_eq!(resolve_gateway_base("10.1.2.3"), "http://10.1.2.3:18088");
    }
}
