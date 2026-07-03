//! Admin global clawTap settings + cluster probe. Author: kejiqing

use serde::{Deserialize, Serialize};

use crate::cluster_identity::{
    fetch_tap_cluster_identity, gateway_cluster_id, gateway_database_url, local_cluster_identity,
    verify_tap_cluster, ClusterIdentity,
};
use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::session_db::GatewaySessionDb;

pub const DEFAULT_CLAW_TAP_PROXY_PORT: u16 = 8080;
pub const DEFAULT_CLAW_TAP_LIVE_PORT: u16 = 3000;
/// Admin session link placeholder (claude-tap traces API).
pub const LIVE_SESSION_ID_PLACEHOLDER: &str = "{sessionId}";

/// Browser Live viewer (Claude Trace UI): `GET /?session=…` on E2B Host traffic URL.
#[must_use]
pub fn live_session_viewer_url_template(live_base_url: &str) -> String {
    let base = live_base_url.trim().trim_end_matches('/');
    format!("{base}/?session={LIVE_SESSION_ID_PLACEHOLDER}")
}

/// JSON API for programmatic trace fetch (not the HTML Live viewer).
#[must_use]
pub fn live_session_traces_url_template(live_base_url: &str) -> String {
    format!(
        "{}/api/sessions/traces?session={LIVE_SESSION_ID_PLACEHOLDER}",
        live_base_url.trim().trim_end_matches('/')
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ClawTapMode {
    #[default]
    Local,
    Remote,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClawTapSettings {
    #[serde(default)]
    pub mode: ClawTapMode,
    #[serde(default)]
    pub host: String,
    #[serde(rename = "proxyPort", default = "default_proxy_port")]
    pub proxy_port: u16,
    #[serde(rename = "livePort", default = "default_live_port")]
    pub live_port: u16,
    #[serde(rename = "updatedAtMs", default)]
    pub updated_at_ms: i64,
    /// e2b observe singleton URLs persisted by `e2b-tap-live-up.py` (Admin reads PG). Author: kejiqing
    #[serde(rename = "liveBaseUrl", default)]
    pub live_base_url: Option<String>,
    #[serde(rename = "liveSessionUrlTemplate", default)]
    pub live_session_url_template: Option<String>,
    #[serde(rename = "proxyBaseUrl", default)]
    pub proxy_base_url: Option<String>,
    #[serde(rename = "e2bObserveSandboxId", default)]
    pub e2b_observe_sandbox_id: Option<String>,
}

fn default_proxy_port() -> u16 {
    DEFAULT_CLAW_TAP_PROXY_PORT
}

fn default_live_port() -> u16 {
    DEFAULT_CLAW_TAP_LIVE_PORT
}

#[derive(Debug, Clone, Serialize)]
pub struct ClawTapSettingsPublic {
    pub mode: ClawTapMode,
    pub host: String,
    #[serde(rename = "proxyPort")]
    pub proxy_port: u16,
    #[serde(rename = "livePort", skip_serializing_if = "Option::is_none")]
    pub live_port: Option<u16>,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    #[serde(rename = "configured")]
    pub configured: bool,
    #[serde(rename = "proxyBaseUrl", skip_serializing_if = "Option::is_none")]
    pub proxy_base_url: Option<String>,
    #[serde(rename = "liveBaseUrl", skip_serializing_if = "Option::is_none")]
    pub live_base_url: Option<String>,
    #[serde(
        rename = "liveSessionUrlTemplate",
        skip_serializing_if = "Option::is_none"
    )]
    pub live_session_url_template: Option<String>,
    /// Self-hosted e2b: add this line to `/etc/hosts` so Live/OVS traffic host resolves.
    #[serde(
        rename = "liveBrowserHostsLine",
        skip_serializing_if = "Option::is_none"
    )]
    pub live_browser_hosts_line: Option<String>,
    /// e2b observe singleton sandbox (`observe-tap-up` writes PG). Author: kejiqing
    #[serde(
        rename = "e2bObserveSandboxId",
        skip_serializing_if = "Option::is_none"
    )]
    pub e2b_observe_sandbox_id: Option<String>,
    /// Live e2b `GET /sandboxes/{id}` state (running / killed / …). Author: kejiqing
    #[serde(
        rename = "e2bObserveSandboxState",
        skip_serializing_if = "Option::is_none"
    )]
    pub e2b_observe_sandbox_state: Option<String>,
    #[serde(
        rename = "e2bObserveSandboxRunning",
        skip_serializing_if = "Option::is_none"
    )]
    pub e2b_observe_sandbox_running: Option<bool>,
    #[serde(
        rename = "e2bObserveSandboxEndAtMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub e2b_observe_sandbox_end_at_ms: Option<i64>,
    #[serde(
        rename = "e2bObserveSandboxRemainingTtlSecs",
        skip_serializing_if = "Option::is_none"
    )]
    pub e2b_observe_sandbox_remaining_ttl_secs: Option<u64>,
}

impl ClawTapSettings {
    /// Legacy rows without `mode`: external host → remote; otherwise local. Author: kejiqing
    pub fn normalize_mode(&mut self) {
        if self.updated_at_ms <= 0 {
            self.mode = ClawTapMode::Local;
            return;
        }
        if self.mode == ClawTapMode::Local
            && !self.host.is_empty()
            && !is_local_internal_host(&self.host)
        {
            self.mode = ClawTapMode::Remote;
        }
    }
}

fn is_local_internal_host(host: &str) -> bool {
    matches!(
        host.trim(),
        "claw-claude-tap"
            | "127.0.0.1"
            | "localhost"
            | "host.containers.internal"
            | "host.docker.internal"
    )
}

/// DNS / compose service name for worker + gateway → tap proxy (local mode). Author: kejiqing
#[must_use]
pub fn resolve_local_tap_internal_host() -> String {
    if let Ok(raw) = std::env::var("CLAW_TAP_INTERNAL_HOST") {
        let h = raw.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    if let Ok(name) = std::env::var("CLAUDE_TAP_CONTAINER_NAME") {
        let h = name.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    "claw-claude-tap".to_string()
}

/// Browser-facing Live viewer host (local mode). Author: kejiqing
#[must_use]
pub fn resolve_local_tap_live_public_host() -> String {
    if let Ok(raw) = std::env::var("CLAW_TAP_LIVE_PUBLIC_HOST") {
        let h = raw.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    if let Ok(raw) = std::env::var("CLAW_POOL_ADVERTISE_HOST") {
        let h = raw.trim();
        if !h.is_empty() {
            return h.to_string();
        }
    }
    "127.0.0.1".to_string()
}

impl From<&ClawTapSettings> for ClawTapSettingsPublic {
    fn from(s: &ClawTapSettings) -> Self {
        if let Some(live) = s.live_base_url.as_ref().filter(|u| !u.trim().is_empty()) {
            let configured = s.updated_at_ms > 0;
            return Self {
                mode: s.mode,
                host: if s.mode == ClawTapMode::Local {
                    String::new()
                } else {
                    s.host.clone()
                },
                proxy_port: normalize_proxy_port(s.proxy_port),
                live_port: Some(normalize_live_port(s.live_port)),
                updated_at_ms: s.updated_at_ms,
                configured,
                proxy_base_url: s.proxy_base_url.clone(),
                live_base_url: Some(live.trim().to_string()),
                live_session_url_template: s.live_session_url_template.clone(),
                live_browser_hosts_line: None,
                e2b_observe_sandbox_id: s.e2b_observe_sandbox_id.clone(),
                e2b_observe_sandbox_state: None,
                e2b_observe_sandbox_running: None,
                e2b_observe_sandbox_end_at_ms: None,
                e2b_observe_sandbox_remaining_ttl_secs: None,
            };
        }
        let configured =
            s.updated_at_ms > 0 && (s.mode == ClawTapMode::Local || !s.host.trim().is_empty());
        let proxy_port = normalize_proxy_port(s.proxy_port);
        let live_port = normalize_live_port(s.live_port);
        let (proxy_base_url, live_base_url, live_session_url_template, live_port_out) =
            if configured {
                match s.mode {
                    ClawTapMode::Local => {
                        let proxy = claw_tap_proxy_base_url(&s.host, proxy_port);
                        let public_host = resolve_local_tap_live_public_host();
                        let live = claw_tap_live_base_url(&public_host, live_port);
                        let template = live.as_ref().map(|b| live_session_viewer_url_template(b));
                        (proxy, live, template, Some(live_port))
                    }
                    ClawTapMode::Remote => {
                        let proxy = claw_tap_proxy_base_url(&s.host, proxy_port);
                        (proxy, None, None, None)
                    }
                }
            } else {
                (None, None, None, None)
            };
        Self {
            mode: s.mode,
            host: if s.mode == ClawTapMode::Local {
                String::new()
            } else {
                s.host.clone()
            },
            proxy_port,
            live_port: live_port_out,
            updated_at_ms: s.updated_at_ms,
            configured,
            proxy_base_url,
            live_base_url,
            live_session_url_template,
            live_browser_hosts_line: None,
            e2b_observe_sandbox_id: s.e2b_observe_sandbox_id.clone(),
            e2b_observe_sandbox_state: None,
            e2b_observe_sandbox_running: None,
            e2b_observe_sandbox_end_at_ms: None,
            e2b_observe_sandbox_remaining_ttl_secs: None,
        }
    }
}

/// Attach live e2b sandbox TTL/state for Admin observe diagnostics. Author: kejiqing
pub async fn enrich_claw_tap_observe_runtime(
    tap: &mut ClawTapSettingsPublic,
    client: &claw_e2b_sandbox_client::E2bSandboxClient,
) {
    let Some(sid) = tap
        .e2b_observe_sandbox_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    if let Ok(snap) = client.fetch_sandbox_snapshot(sid).await {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let running = snap.is_running();
        let end_at_ms = snap.end_at_ms;
        let remaining = snap.remaining_ttl_secs(now_ms);
        tap.e2b_observe_sandbox_state = Some(snap.state);
        tap.e2b_observe_sandbox_running = Some(running);
        tap.e2b_observe_sandbox_end_at_ms = end_at_ms;
        tap.e2b_observe_sandbox_remaining_ttl_secs = remaining;
    } else {
        tap.e2b_observe_sandbox_state = Some("unknown".into());
        tap.e2b_observe_sandbox_running = Some(false);
    }
}

#[derive(Debug, Deserialize)]
pub struct PutClawTapSettingsInput {
    #[serde(default)]
    pub mode: ClawTapMode,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(rename = "proxyPort", default)]
    pub proxy_port: Option<u16>,
    #[serde(rename = "livePort", default)]
    pub live_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PutClawTapSettingsResponse {
    #[serde(flatten)]
    pub settings: ClawTapSettingsPublic,
    #[serde(rename = "tapRestart", skip_serializing_if = "Option::is_none")]
    pub tap_restart: Option<crate::gateway_claw_tap_lifecycle::TapRestartOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProbeClawTapInput {
    #[serde(default)]
    pub mode: ClawTapMode,
    #[serde(default)]
    pub host: Option<String>,
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

pub fn normalize_live_port(port: u16) -> u16 {
    if port == 0 {
        DEFAULT_CLAW_TAP_LIVE_PORT
    } else {
        port
    }
}

pub fn claw_tap_proxy_base_url(host: &str, proxy_port: u16) -> Option<String> {
    let h = normalize_claw_tap_host(host)?;
    let port = normalize_proxy_port(proxy_port);
    Some(format!("http://{h}:{port}"))
}

pub fn claw_tap_live_base_url(host: &str, live_port: u16) -> Option<String> {
    let h = normalize_claw_tap_host(host)?;
    let port = normalize_live_port(live_port);
    Some(format!("http://{h}:{port}"))
}

pub async fn load_claw_tap_public(
    db: &GatewaySessionDb,
) -> Result<ClawTapSettingsPublic, sqlx::Error> {
    let (mut settings, _, _) = get_gateway_global_settings(db).await?;
    settings.claw_tap.normalize_mode();
    Ok(ClawTapSettingsPublic::from(&settings.claw_tap))
}

/// e2b mode: Admin session traces **only** via e2b observe — never compose `192.168.x:3000`.
#[must_use]
pub fn strip_compose_live_urls_for_fc_admin(
    mut tap: ClawTapSettingsPublic,
) -> ClawTapSettingsPublic {
    tap.live_base_url = None;
    tap.live_session_url_template = None;
    tap.live_port = None;
    tap
}

fn local_identity_for_settings() -> Result<ClusterIdentity, String> {
    let cluster_id = gateway_cluster_id()?;
    let db_url = gateway_database_url()?;
    local_cluster_identity(&cluster_id, &db_url)
}

async fn probe_host_port(host: &str, proxy_port: u16) -> ProbeClawTapResponse {
    let Some(host) = normalize_claw_tap_host(host) else {
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
    let port = normalize_proxy_port(proxy_port);
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

pub async fn probe_claw_tap_endpoint(
    _db: &GatewaySessionDb,
    input: ProbeClawTapInput,
) -> ProbeClawTapResponse {
    let host = match input.mode {
        ClawTapMode::Local => resolve_local_tap_internal_host(),
        ClawTapMode::Remote => input
            .host
            .as_deref()
            .and_then(normalize_claw_tap_host)
            .unwrap_or_default(),
    };
    if host.is_empty() {
        return ProbeClawTapResponse {
            ok: false,
            message: "clawTap host is required for remote mode".into(),
            probe_url: String::new(),
            cluster_id: None,
            db_host: None,
            cluster_hash: None,
            local_cluster_hash: None,
            cluster_match: None,
            hash_match: None,
            latency_ms: None,
        };
    }
    let proxy_port = match input.mode {
        ClawTapMode::Local => DEFAULT_CLAW_TAP_PROXY_PORT,
        ClawTapMode::Remote => normalize_proxy_port(input.proxy_port),
    };
    probe_host_port(&host, proxy_port).await
}

pub async fn put_claw_tap_settings(
    db: &GatewaySessionDb,
    input: PutClawTapSettingsInput,
) -> Result<PutClawTapSettingsResponse, String> {
    let (host, proxy_port, live_port, probe_required) = match input.mode {
        ClawTapMode::Local => {
            let host = resolve_local_tap_internal_host();
            let live_port =
                normalize_live_port(input.live_port.unwrap_or(DEFAULT_CLAW_TAP_LIVE_PORT));
            (host, DEFAULT_CLAW_TAP_PROXY_PORT, live_port, false)
        }
        ClawTapMode::Remote => {
            let host = input
                .host
                .as_deref()
                .and_then(normalize_claw_tap_host)
                .ok_or_else(|| "clawTap host is required for remote mode".to_string())?;
            let proxy_port =
                normalize_proxy_port(input.proxy_port.unwrap_or(DEFAULT_CLAW_TAP_PROXY_PORT));
            (host, proxy_port, DEFAULT_CLAW_TAP_LIVE_PORT, true)
        }
    };

    claw_tap_proxy_base_url(&host, proxy_port)
        .ok_or_else(|| "invalid clawTap host/port".to_string())?;

    let probe = probe_claw_tap_endpoint(
        db,
        ProbeClawTapInput {
            mode: input.mode,
            host: Some(host.clone()),
            proxy_port,
        },
    )
    .await;

    if probe_required && !probe.ok {
        return Err(probe.message);
    }

    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    settings.claw_tap = ClawTapSettings {
        mode: input.mode,
        host,
        proxy_port,
        live_port,
        updated_at_ms: now_ms(),
        ..Default::default()
    };
    save_gateway_global_settings(db, &settings, &tokens, now_ms())
        .await
        .map_err(|e| e.to_string())?;

    let mut message = None;
    if !probe.ok {
        message = Some(format!(
            "clawTap saved; tap not reachable yet ({}) — ensure pool tap: gateway.sh pool-up or up",
            probe.message
        ));
    }

    Ok(PutClawTapSettingsResponse {
        settings: ClawTapSettingsPublic::from(&settings.claw_tap),
        tap_restart: None,
        message,
    })
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
    fn proxy_and_live_base_urls() {
        assert_eq!(
            claw_tap_proxy_base_url("192.168.1.10", 8080).as_deref(),
            Some("http://192.168.1.10:8080")
        );
        assert_eq!(
            claw_tap_live_base_url("192.168.9.252", 3000).as_deref(),
            Some("http://192.168.9.252:3000")
        );
    }

    #[test]
    fn legacy_remote_inferred_from_external_host() {
        let mut s = ClawTapSettings {
            mode: ClawTapMode::Local,
            host: "10.22.28.94".into(),
            proxy_port: 8081,
            live_port: 3000,
            updated_at_ms: 1,
            ..Default::default()
        };
        s.normalize_mode();
        assert_eq!(s.mode, ClawTapMode::Remote);
    }

    #[test]
    fn local_public_uses_live_public_host() {
        std::env::set_var("CLAW_TAP_LIVE_PUBLIC_HOST", "127.0.0.1");
        let s = ClawTapSettings {
            mode: ClawTapMode::Local,
            host: "claw-claude-tap".into(),
            proxy_port: 8080,
            live_port: 3000,
            updated_at_ms: 1,
            ..Default::default()
        };
        let pub_ = ClawTapSettingsPublic::from(&s);
        assert!(pub_.configured);
        assert_eq!(pub_.host, "");
        assert_eq!(pub_.live_port, Some(3000));
        assert_eq!(pub_.live_base_url.as_deref(), Some("http://127.0.0.1:3000"));
        std::env::remove_var("CLAW_TAP_LIVE_PUBLIC_HOST");
    }

    #[test]
    fn remote_public_omits_live_urls() {
        let s = ClawTapSettings {
            mode: ClawTapMode::Remote,
            host: "10.22.28.94".into(),
            proxy_port: 8081,
            live_port: 3000,
            updated_at_ms: 1,
            ..Default::default()
        };
        let pub_ = ClawTapSettingsPublic::from(&s);
        assert_eq!(pub_.host, "10.22.28.94");
        assert!(pub_.live_port.is_none());
        assert!(pub_.live_base_url.is_none());
    }

    #[test]
    fn from_persisted_e2b_observe_urls() {
        let s = ClawTapSettings {
            mode: ClawTapMode::Remote,
            host: "8080-sbx_abc.supone.top".into(),
            proxy_port: 8080,
            live_port: 3000,
            updated_at_ms: 1_700_000_000_000,
            live_base_url: Some("http://3000-sbx_abc.supone.top".into()),
            live_session_url_template: Some(
                "http://3000-sbx_abc.supone.top/?session={sessionId}".into(),
            ),
            proxy_base_url: Some("http://8080-sbx_abc.supone.top".into()),
            e2b_observe_sandbox_id: Some("sbx_abc".into()),
        };
        let pub_ = ClawTapSettingsPublic::from(&s);
        assert!(pub_.configured);
        assert_eq!(
            pub_.live_base_url.as_deref(),
            Some("http://3000-sbx_abc.supone.top")
        );
        assert_eq!(
            pub_.proxy_base_url.as_deref(),
            Some("http://8080-sbx_abc.supone.top")
        );
    }

    #[test]
    fn live_session_traces_api_template() {
        assert_eq!(
            live_session_traces_url_template("http://192.168.125.115:3000"),
            "http://192.168.125.115:3000/api/sessions/traces?session={sessionId}"
        );
    }

    #[test]
    fn live_session_viewer_url_template_for_admin() {
        assert_eq!(
            live_session_viewer_url_template("http://192.168.125.115:3000"),
            "http://192.168.125.115:3000/?session={sessionId}"
        );
        assert_eq!(
            live_session_viewer_url_template("http://3000-sbx_abc.supone.top"),
            "http://3000-sbx_abc.supone.top/?session={sessionId}"
        );
    }
}
