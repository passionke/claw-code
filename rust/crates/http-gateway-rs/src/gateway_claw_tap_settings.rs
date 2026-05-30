//! Admin global clawTap settings + cluster probe. Author: kejiqing

use serde::{Deserialize, Serialize};

use crate::cluster_identity::{
    fetch_tap_cluster_identity, gateway_cluster_id, gateway_database_url, local_cluster_identity,
    verify_tap_cluster, ClusterIdentity,
};
use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::session_db::GatewaySessionDb;

pub const DEFAULT_CLAW_TAP_PROXY_PORT: u16 = 8080;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClawTapSettings {
    #[serde(default)]
    pub host: String,
    #[serde(rename = "proxyPort", default = "default_proxy_port")]
    pub proxy_port: u16,
    #[serde(rename = "updatedAtMs", default)]
    pub updated_at_ms: i64,
}

fn default_proxy_port() -> u16 {
    DEFAULT_CLAW_TAP_PROXY_PORT
}

#[derive(Debug, Clone, Serialize)]
pub struct ClawTapSettingsPublic {
    pub host: String,
    #[serde(rename = "proxyPort")]
    pub proxy_port: u16,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    #[serde(rename = "configured")]
    pub configured: bool,
}

impl From<&ClawTapSettings> for ClawTapSettingsPublic {
    fn from(s: &ClawTapSettings) -> Self {
        Self {
            host: s.host.clone(),
            proxy_port: if s.proxy_port == 0 {
                DEFAULT_CLAW_TAP_PROXY_PORT
            } else {
                s.proxy_port
            },
            updated_at_ms: s.updated_at_ms,
            configured: s.updated_at_ms > 0 && !s.host.trim().is_empty(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PutClawTapSettingsInput {
    pub host: String,
    #[serde(rename = "proxyPort", default = "default_proxy_port")]
    pub proxy_port: u16,
}

#[derive(Debug, Deserialize)]
pub struct ProbeClawTapInput {
    pub host: String,
    #[serde(rename = "proxyPort", default = "default_proxy_port")]
    pub proxy_port: u16,
}

#[derive(Debug, Serialize)]
pub struct ProbeClawTapResponse {
    pub ok: bool,
    pub message: String,
    #[serde(rename = "probeUrl")]
    pub probe_url: String,
    #[serde(rename = "clusterId", skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    #[serde(rename = "dbHost", skip_serializing_if = "Option::is_none")]
    pub db_host: Option<String>,
    #[serde(rename = "clusterHash", skip_serializing_if = "Option::is_none")]
    pub cluster_hash: Option<String>,
    #[serde(rename = "localClusterHash", skip_serializing_if = "Option::is_none")]
    pub local_cluster_hash: Option<String>,
    #[serde(rename = "clusterMatch", skip_serializing_if = "Option::is_none")]
    pub cluster_match: Option<bool>,
    #[serde(rename = "hashMatch", skip_serializing_if = "Option::is_none")]
    pub hash_match: Option<bool>,
    #[serde(rename = "latencyMs", skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

pub fn normalize_claw_tap_host(raw: &str) -> Option<String> {
    let mut s = raw.trim();
    if s.is_empty() {
        return None;
    }
    for prefix in ["http://", "https://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest;
            break;
        }
    }
    let host = s.split('/').next()?.split(':').next()?.trim();
    if host.is_empty() || host.len() > 253 {
        return None;
    }
    if host.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    Some(host.to_string())
}

pub fn normalize_proxy_port(port: u16) -> u16 {
    if port == 0 {
        DEFAULT_CLAW_TAP_PROXY_PORT
    } else {
        port
    }
}

pub fn claw_tap_proxy_base_url(host: &str, proxy_port: u16) -> Option<String> {
    let h = normalize_claw_tap_host(host)?;
    let port = normalize_proxy_port(proxy_port);
    Some(format!("http://{h}:{port}"))
}

pub async fn load_claw_tap_public(
    db: &GatewaySessionDb,
) -> Result<ClawTapSettingsPublic, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(ClawTapSettingsPublic::from(&settings.claw_tap))
}

fn local_identity_for_settings() -> Result<ClusterIdentity, String> {
    let cluster_id = gateway_cluster_id()?;
    let db_url = gateway_database_url()?;
    local_cluster_identity(&cluster_id, &db_url)
}

pub async fn probe_claw_tap_endpoint(
    _db: &GatewaySessionDb,
    input: ProbeClawTapInput,
) -> ProbeClawTapResponse {
    let Some(host) = normalize_claw_tap_host(&input.host) else {
        return ProbeClawTapResponse {
            ok: false,
            message: "invalid host".into(),
            probe_url: String::new(),
            cluster_id: None,
            db_host: None,
            cluster_hash: None,
            local_cluster_hash: None,
            cluster_match: None,
            hash_match: None,
            latency_ms: None,
        };
    };
    let port = normalize_proxy_port(input.proxy_port);
    let base = format!("http://{host}:{port}");
    let probe_url = format!("{base}/healthz");
    let local = match local_identity_for_settings() {
        Ok(v) => v,
        Err(e) => {
            return ProbeClawTapResponse {
                ok: false,
                message: e,
                probe_url,
                cluster_id: None,
                db_host: None,
                cluster_hash: None,
                local_cluster_hash: None,
                cluster_match: None,
                hash_match: None,
                latency_ms: None,
            };
        }
    };
    let started = std::time::Instant::now();
    let expected_cluster_id = match gateway_cluster_id() {
        Ok(v) => v,
        Err(e) => {
            return ProbeClawTapResponse {
                ok: false,
                message: e,
                probe_url,
                cluster_id: None,
                db_host: None,
                cluster_hash: None,
                local_cluster_hash: Some(local.cluster_hash.clone()),
                cluster_match: None,
                hash_match: None,
                latency_ms: None,
            };
        }
    };
    match fetch_tap_cluster_identity(&base, &expected_cluster_id).await {
        Ok(tap) => {
            let latency_ms = u64::try_from(started.elapsed().as_millis()).ok();
            let cluster_match = tap.cluster_id == local.cluster_id;
            let hash_match = tap.cluster_hash == local.cluster_hash;
            let verify = verify_tap_cluster(&local, &tap);
            let ok = verify.is_ok();
            let message = if ok {
                "clawTap cluster identity verified".into()
            } else {
                verify.err().map(|e| e.message).unwrap_or_default()
            };
            ProbeClawTapResponse {
                ok,
                message,
                probe_url,
                cluster_id: Some(tap.cluster_id),
                db_host: (!tap.db_host.is_empty()).then(|| tap.db_host.clone()),
                cluster_hash: Some(tap.cluster_hash),
                local_cluster_hash: Some(local.cluster_hash),
                cluster_match: Some(cluster_match),
                hash_match: Some(hash_match),
                latency_ms,
            }
        }
        Err(e) => ProbeClawTapResponse {
            ok: false,
            message: e,
            probe_url,
            cluster_id: None,
            db_host: None,
            cluster_hash: None,
            local_cluster_hash: Some(local.cluster_hash),
            cluster_match: None,
            hash_match: None,
            latency_ms: u64::try_from(started.elapsed().as_millis()).ok(),
        },
    }
}

pub async fn put_claw_tap_settings(
    db: &GatewaySessionDb,
    input: PutClawTapSettingsInput,
) -> Result<ClawTapSettingsPublic, String> {
    let host = normalize_claw_tap_host(&input.host)
        .ok_or_else(|| "clawTap host is required".to_string())?;
    let proxy_port = normalize_proxy_port(input.proxy_port);
    claw_tap_proxy_base_url(&host, proxy_port)
        .ok_or_else(|| "invalid clawTap host/port".to_string())?;
    let probe = probe_claw_tap_endpoint(
        db,
        ProbeClawTapInput {
            host: input.host.clone(),
            proxy_port: input.proxy_port,
        },
    )
    .await;
    if !probe.ok {
        return Err(probe.message);
    }
    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    settings.claw_tap = ClawTapSettings {
        host,
        proxy_port,
        updated_at_ms: now_ms(),
    };
    save_gateway_global_settings(db, &settings, &tokens, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    Ok(ClawTapSettingsPublic::from(&settings.claw_tap))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_host_strips_scheme() {
        assert_eq!(
            normalize_claw_tap_host("http://10.0.0.5:8080/path").as_deref(),
            Some("10.0.0.5")
        );
    }

    #[test]
    fn proxy_base_url() {
        assert_eq!(
            claw_tap_proxy_base_url("192.168.1.10", 8080).as_deref(),
            Some("http://192.168.1.10:8080")
        );
    }
}
