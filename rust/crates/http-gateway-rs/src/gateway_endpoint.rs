//! Per-cluster gateway HTTP endpoint registry (multi-gateway same clusterId). Author: kejiqing
//!
//! Semantics: this is a **gateway ingress** registry, not a pool/worker registry.
//! Each gateway process upserts itself at startup and heartbeats; Admin lists online rows
//! plus offline rows with heartbeat within 24h under the shared `CLAW_CLUSTER_ID`.

use serde::Serialize;

use crate::session_db::GatewaySessionDb;

/// Online if heartbeat within this window (ms). Author: kejiqing
pub const GATEWAY_ENDPOINT_ONLINE_WINDOW_MS: i64 = 90_000;
/// Admin list: offline rows kept while last heartbeat is within this window (ms). Author: kejiqing
pub const GATEWAY_ENDPOINT_LIST_OFFLINE_RETENTION_MS: i64 = 86_400_000;
/// Background heartbeat interval. Author: kejiqing
pub const GATEWAY_ENDPOINT_HEARTBEAT_INTERVAL_SECS: u64 = 30;

const GATEWAY_ID_ENV: &str = "CLAW_GATEWAY_ID";
const GATEWAY_BASE_ENV: &str = "CLAW_GATEWAY_BASE";

#[derive(Debug, Clone)]
pub struct GatewayEndpointIdentity {
    pub gateway_id: String,
    pub gateway_base: String,
    pub hostname: String,
}

#[derive(Debug, Clone)]
pub struct GatewayEndpointRow {
    pub gateway_id: String,
    pub gateway_base: String,
    pub hostname: String,
    pub started_at_ms: i64,
    pub last_heartbeat_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayEndpointPublic {
    pub gateway_id: String,
    pub gateway_base: String,
    pub hostname: String,
    pub started_at_ms: i64,
    pub last_heartbeat_ms: i64,
    pub online: bool,
    #[serde(rename = "self")]
    pub is_self: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayEndpointsResponse {
    pub cluster_id: String,
    pub self_gateway_id: String,
    pub self_gateway_base: String,
    pub endpoints: Vec<GatewayEndpointPublic>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn hostname_fallback() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

fn validate_gateway_id(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 64 {
        return Err("CLAW_GATEWAY_ID must be 1..=64 chars".into());
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("CLAW_GATEWAY_ID must match [A-Za-z0-9_-]+".into());
    }
    Ok(s.to_string())
}

fn normalize_gateway_base(raw: &str) -> Result<String, String> {
    let s = raw.trim().trim_end_matches('/').to_string();
    if s.is_empty() {
        return Err("gateway_base is empty".into());
    }
    if !(s.starts_with("http://") || s.starts_with("https://")) {
        return Err(format!("gateway_base must be http(s) URL, got {s}"));
    }
    Ok(s)
}

/// Resolve this process's gateway ingress identity from env. Author: kejiqing
pub fn resolve_gateway_endpoint_identity() -> Result<GatewayEndpointIdentity, String> {
    let hostname = hostname_fallback();
    let gateway_id = if let Ok(raw) = std::env::var(GATEWAY_ID_ENV) {
        validate_gateway_id(&raw)?
    } else {
        validate_gateway_id(&format!("gw-{hostname}"))?
    };

    let gateway_base = if let Ok(raw) = std::env::var(GATEWAY_BASE_ENV) {
        normalize_gateway_base(&raw)?
    } else {
        let host = std::env::var("CLAW_POOL_ADVERTISE_HOST")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| hostname.clone());
        let port = std::env::var("GATEWAY_HOST_PORT")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "18088".to_string());
        normalize_gateway_base(&format!("http://{host}:{port}"))?
    };

    Ok(GatewayEndpointIdentity {
        gateway_id,
        gateway_base,
        hostname,
    })
}

#[must_use]
pub fn is_gateway_endpoint_online(last_heartbeat_ms: i64, now_ms: i64) -> bool {
    last_heartbeat_ms > 0
        && now_ms.saturating_sub(last_heartbeat_ms) <= GATEWAY_ENDPOINT_ONLINE_WINDOW_MS
}

/// Admin `GET /v1/gateway/endpoints`: all online + offline with heartbeat within 24h. Author: kejiqing
#[must_use]
pub fn should_list_gateway_endpoint(last_heartbeat_ms: i64, now_ms: i64) -> bool {
    if last_heartbeat_ms <= 0 {
        return false;
    }
    if is_gateway_endpoint_online(last_heartbeat_ms, now_ms) {
        return true;
    }
    now_ms.saturating_sub(last_heartbeat_ms) <= GATEWAY_ENDPOINT_LIST_OFFLINE_RETENTION_MS
}

/// Register this gateway and spawn heartbeat ticker. Author: kejiqing
pub async fn register_and_spawn_heartbeat(
    db: std::sync::Arc<GatewaySessionDb>,
    identity: GatewayEndpointIdentity,
) -> Result<(), String> {
    let started = now_ms();
    db.upsert_gateway_endpoint(&identity, started, started)
        .await
        .map_err(|e| format!("upsert gateway_endpoint: {e}"))?;
    tracing::info!(
        target: "claw_gateway_endpoint",
        gateway_id = %identity.gateway_id,
        gateway_base = %identity.gateway_base,
        "gateway_endpoint registered"
    );
    let hb_id = identity.gateway_id.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            GATEWAY_ENDPOINT_HEARTBEAT_INTERVAL_SECS,
        ));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let ts = now_ms();
            if let Err(e) = db.touch_gateway_endpoint_heartbeat(&hb_id, ts).await {
                tracing::warn!(
                    target: "claw_gateway_endpoint",
                    gateway_id = %hb_id,
                    error = %e,
                    "gateway_endpoint heartbeat failed"
                );
            }
        }
    });
    Ok(())
}

pub async fn list_endpoints_response(
    db: &GatewaySessionDb,
    self_identity: &GatewayEndpointIdentity,
) -> Result<GatewayEndpointsResponse, String> {
    let now = now_ms();
    let rows = db
        .list_gateway_endpoints()
        .await
        .map_err(|e| e.to_string())?;
    let endpoints = rows
        .into_iter()
        .filter(|r| should_list_gateway_endpoint(r.last_heartbeat_ms, now))
        .map(|r| GatewayEndpointPublic {
            online: is_gateway_endpoint_online(r.last_heartbeat_ms, now),
            is_self: r.gateway_id == self_identity.gateway_id,
            gateway_id: r.gateway_id,
            gateway_base: r.gateway_base,
            hostname: r.hostname,
            started_at_ms: r.started_at_ms,
            last_heartbeat_ms: r.last_heartbeat_ms,
        })
        .collect();
    Ok(GatewayEndpointsResponse {
        cluster_id: db.cluster_id().to_string(),
        self_gateway_id: self_identity.gateway_id.clone(),
        self_gateway_base: self_identity.gateway_base.clone(),
        endpoints,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_gateway_id_ok() {
        assert!(validate_gateway_id("gw-prod-01").is_ok());
        assert!(validate_gateway_id("").is_err());
        assert!(validate_gateway_id("bad id").is_err());
    }

    #[test]
    fn normalize_base_strips_slash() {
        assert_eq!(
            normalize_gateway_base("http://10.0.0.1:18088/").unwrap(),
            "http://10.0.0.1:18088"
        );
    }

    #[test]
    fn online_window() {
        assert!(is_gateway_endpoint_online(
            1000,
            1000 + GATEWAY_ENDPOINT_ONLINE_WINDOW_MS
        ));
        assert!(!is_gateway_endpoint_online(
            1000,
            1000 + GATEWAY_ENDPOINT_ONLINE_WINDOW_MS + 1
        ));
    }

    #[test]
    fn list_filter_keeps_online_and_recent_offline() {
        let now = 1_000_000_000_i64;
        assert!(should_list_gateway_endpoint(
            now - GATEWAY_ENDPOINT_ONLINE_WINDOW_MS,
            now
        ));
        assert!(should_list_gateway_endpoint(
            now - GATEWAY_ENDPOINT_LIST_OFFLINE_RETENTION_MS,
            now
        ));
        assert!(should_list_gateway_endpoint(
            now - GATEWAY_ENDPOINT_LIST_OFFLINE_RETENTION_MS + 1,
            now
        ));
        assert!(!should_list_gateway_endpoint(
            now - GATEWAY_ENDPOINT_LIST_OFFLINE_RETENTION_MS - 1,
            now
        ));
        assert!(!should_list_gateway_endpoint(0, now));
    }
}
