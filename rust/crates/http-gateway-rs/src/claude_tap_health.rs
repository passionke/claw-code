//! Claude-tap URLs for `/healthz` (proxy + Live viewer). Author: kejiqing

use serde_json::{json, Value};

const DEFAULT_TAP_PROXY_PORT: u16 = 8080;
const DEFAULT_TAP_LIVE_PORT: u16 = 3000;
const LIVE_SESSION_QUERY_PARAM: &str = "session";

/// Snapshot exposed under `healthz.claudeTap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeTapHealthSnapshot {
    pub internal_proxy_base_url: Option<String>,
    pub public_proxy_base_url: String,
    pub public_live_base_url: String,
    pub live_session_query_param: &'static str,
    pub live_session_url_template: String,
    pub tap_proxy_port: u16,
    pub tap_live_port: u16,
    pub gateway_public_base_url: Option<String>,
}

pub fn claude_tap_health_json(request_host: Option<&str>) -> Value {
    let snap = build_claude_tap_health(request_host);
    json!({
        "internalProxyBaseUrl": snap.internal_proxy_base_url,
        "publicProxyBaseUrl": snap.public_proxy_base_url,
        "publicLiveBaseUrl": snap.public_live_base_url,
        "liveSessionQueryParam": snap.live_session_query_param,
        "liveSessionUrlTemplate": snap.live_session_url_template,
        "tapProxyPort": snap.tap_proxy_port,
        "tapLivePort": snap.tap_live_port,
        "gatewayPublicBaseUrl": snap.gateway_public_base_url,
    })
}

pub fn build_claude_tap_health(request_host: Option<&str>) -> ClaudeTapHealthSnapshot {
    let proxy_port = env_u16(&["CLAUDE_TAP_HOST_PORT", "CLAUDE_TAP_PORT"], DEFAULT_TAP_PROXY_PORT);
    let live_port = env_u16(&["CLAUDE_TAP_LIVE_PORT"], DEFAULT_TAP_LIVE_PORT);
    let internal_proxy = env_nonempty("INTERNAL_CLAUDE_TAP_HOST");

    let gateway_public = env_nonempty("CLAW_GATEWAY_PUBLIC_BASE_URL");
    let (scheme, hostname) = resolve_public_origin(request_host, gateway_public.as_deref());

    let public_proxy = format!("{scheme}://{hostname}:{proxy_port}");
    let public_live = format!("{scheme}://{hostname}:{live_port}");
    let live_template = format!(
        "{public_live}/?{LIVE_SESSION_QUERY_PARAM}={{sessionId}}"
    );

    ClaudeTapHealthSnapshot {
        internal_proxy_base_url: internal_proxy,
        public_proxy_base_url: public_proxy,
        public_live_base_url: public_live,
        live_session_query_param: LIVE_SESSION_QUERY_PARAM,
        live_session_url_template: live_template,
        tap_proxy_port: proxy_port,
        tap_live_port: live_port,
        gateway_public_base_url: gateway_public,
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_u16(keys: &[&str], default: u16) -> u16 {
    for key in keys {
        if let Ok(v) = std::env::var(key) {
            if let Ok(n) = v.trim().parse::<u16>() {
                if n > 0 {
                    return n;
                }
            }
        }
    }
    default
}

/// `(scheme, hostname)` for browser-facing tap URLs (no path/port).
fn resolve_public_origin(request_host: Option<&str>, gateway_public: Option<&str>) -> (String, String) {
    if let Some(base) = gateway_public {
        if let Some((scheme, host)) = origin_from_gateway_public_base(base) {
            return (scheme, host);
        }
    }

    if let Some(host_header) = request_host {
        if let Some((scheme, hostname)) = origin_from_host_header(host_header) {
            return (scheme, hostname);
        }
    }

    let fallback_host = env_nonempty("CLAUDE_TAP_PUBLIC_HOST")
        .or_else(|| env_nonempty("CLAUDE_TAP_BIND_HOST"))
        .unwrap_or_else(|| "127.0.0.1".to_string());
    // host.docker.internal is not reachable from a normal browser.
    let hostname = if fallback_host == "host.docker.internal"
        || fallback_host == "host.containers.internal"
    {
        "127.0.0.1".to_string()
    } else {
        fallback_host
    };
    ("http".to_string(), hostname)
}

/// Strip gateway port from `Host` (e.g. `192.168.1.10:18088` → hostname `192.168.1.10`).
fn origin_from_gateway_public_base(base: &str) -> Option<(String, String)> {
    let trimmed = base.trim().trim_end_matches('/');
    let (scheme, rest) = trimmed.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let scheme = if scheme == "http" || scheme == "https" {
        scheme
    } else {
        "http".to_string()
    };
    let authority = rest.split('/').next()?.split('@').next()?.trim();
    if authority.is_empty() {
        return None;
    }
    let hostname = authority
        .rsplit_once(':')
        .map(|(h, _)| h)
        .unwrap_or(authority)
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();
    if hostname.is_empty() {
        return None;
    }
    Some((scheme, hostname))
}

fn origin_from_host_header(host_header: &str) -> Option<(String, String)> {
    let trimmed = host_header.trim();
    if trimmed.is_empty() {
        return None;
    }
    let hostname = trimmed
        .split_once(':')
        .map(|(h, _)| h)
        .unwrap_or(trimmed)
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();
    if hostname.is_empty() {
        return None;
    }
    let scheme = env_nonempty("CLAW_GATEWAY_PUBLIC_SCHEME").unwrap_or_else(|| "http".to_string());
    Some((scheme, hostname))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_live_from_gateway_public_base() {
        let snap = build_claude_tap_health_for_test(
            Some("http://192.168.9.252:18088"),
            None,
            8080,
            3000,
        );
        assert_eq!(snap.public_live_base_url, "http://192.168.9.252:3000");
        assert_eq!(snap.public_proxy_base_url, "http://192.168.9.252:8080");
        assert!(snap
            .live_session_url_template
            .contains("192.168.9.252:3000/?session={sessionId}"));
    }

    #[test]
    fn derives_from_host_header_when_no_public_base() {
        let snap = build_claude_tap_health_for_test(None, Some("10.0.0.5:18088"), 8088, 3000);
        assert_eq!(snap.public_live_base_url, "http://10.0.0.5:3000");
        assert_eq!(snap.public_proxy_base_url, "http://10.0.0.5:8088");
    }

    #[test]
    fn localhost_in_public_base_and_host_header() {
        let from_base = build_claude_tap_health_for_test(
            Some("http://localhost:18088"),
            None,
            8080,
            3000,
        );
        assert_eq!(from_base.public_live_base_url, "http://localhost:3000");

        let from_host = build_claude_tap_health_for_test(None, Some("localhost:18088"), 8080, 3000);
        assert_eq!(from_host.public_live_base_url, "http://localhost:3000");

        let loopback = build_claude_tap_health_for_test(
            Some("http://127.0.0.1:8088"),
            None,
            8080,
            3000,
        );
        assert_eq!(loopback.public_live_base_url, "http://127.0.0.1:3000");
    }

    fn build_claude_tap_health_for_test(
        gateway_public: Option<&str>,
        request_host: Option<&str>,
        proxy_port: u16,
        live_port: u16,
    ) -> ClaudeTapHealthSnapshot {
        let (scheme, hostname) = resolve_public_origin(request_host, gateway_public);
        let public_proxy = format!("{scheme}://{hostname}:{proxy_port}");
        let public_live = format!("{scheme}://{hostname}:{live_port}");
        ClaudeTapHealthSnapshot {
            internal_proxy_base_url: Some("http://host.docker.internal:8080".into()),
            public_proxy_base_url: public_proxy,
            public_live_base_url: public_live.clone(),
            live_session_query_param: LIVE_SESSION_QUERY_PARAM,
            live_session_url_template: format!(
                "{public_live}/?{LIVE_SESSION_QUERY_PARAM}={{sessionId}}"
            ),
            tap_proxy_port: proxy_port,
            tap_live_port: live_port,
            gateway_public_base_url: gateway_public.map(str::to_string),
        }
    }
}
