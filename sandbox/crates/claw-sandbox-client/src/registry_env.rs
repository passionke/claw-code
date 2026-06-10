//! Env-based pool / sandbox identity (no DB). Author: kejiqing

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
