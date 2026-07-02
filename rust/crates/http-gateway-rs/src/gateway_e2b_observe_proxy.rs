//! e2b browser traffic URLs — E2B Host domain (`{port}-{sandboxId}.{domain}`). No gateway HTTP proxy.
//! Spec: e2bserver Affine「Traffic 访问说明」(supone.top). Author: kejiqing

use crate::gateway_claw_tap_settings::{live_session_viewer_url_template, ClawTapSettingsPublic};

/// Self-hosted e2b traffic hostnames use `{port}-sbx_*.{domain}` (SDK `getHost`).
#[must_use]
pub fn should_use_e2b_traffic_browser_proxy(internal_base: &str) -> bool {
    internal_base.contains("-sbx")
}

/// Back-compat alias.
#[must_use]
pub fn should_use_fc_observe_browser_proxy(internal_live_base: &str) -> bool {
    should_use_e2b_traffic_browser_proxy(internal_live_base)
}

/// Parse SDK internal base `http://{port}-{sandboxId}.{domain}` → `(port, sandboxId)`.
pub fn parse_e2b_traffic_identity(internal_base: &str) -> Result<(u16, String), String> {
    let host = traffic_host(internal_base)?;
    let (port_str, rest) = host
        .split_once('-')
        .ok_or_else(|| format!("fc traffic host missing port-sandbox split: {host}"))?;
    let port: u16 = port_str
        .parse()
        .map_err(|e| format!("fc traffic port parse: {e}"))?;
    let sandbox_id = rest.split('.').next().unwrap_or(rest).trim().to_string();
    if sandbox_id.is_empty() {
        return Err("fc traffic sandbox id empty".into());
    }
    Ok((port, sandbox_id))
}

/// Legacy: Host-domain routing needs no `/etc/hosts` when wildcard DNS resolves `*.supone.top`.
pub fn e2b_traffic_browser_hosts_line(
    _internal_base: &str,
    _fc_domain: &str,
) -> Result<String, String> {
    Err("e2b Host traffic: use wildcard DNS (no hosts line)".into())
}

pub fn fc_ovs_browser_hosts_line(
    internal_ovs_base: &str,
    fc_domain: &str,
) -> Result<String, String> {
    e2b_traffic_browser_hosts_line(internal_ovs_base, fc_domain)
}

/// Traffic proxy listen port on e2b host (nginx → :3001); gateway internal probes only.
#[must_use]
pub fn e2b_traffic_proxy_port() -> u16 {
    std::env::var("CLAW_E2B_TRAFFIC_PORT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(3001)
}

fn traffic_host(internal_base: &str) -> Result<String, String> {
    let s = internal_base.trim().trim_end_matches('/');
    let rest = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))
        .ok_or_else(|| "fc traffic url missing http(s) scheme".to_string())?;
    let host = rest
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .trim();
    if host.is_empty() {
        return Err("fc traffic url missing host".into());
    }
    Ok(host.to_string())
}

/// In-sandbox probes via traffic proxy Host header (gateway internal only).
pub fn self_hosted_observe_upstream(
    internal_live_base: &str,
    fc_domain: &str,
) -> Result<(String, String), String> {
    let traffic_host = traffic_host(internal_live_base)?;
    let domain = fc_domain.trim().trim_end_matches('/');
    if domain.is_empty() {
        return Err("CLAW_E2B_DOMAIN empty".into());
    }
    let port = e2b_traffic_proxy_port();
    Ok((traffic_host, format!("http://{domain}:{port}")))
}

/// e2b observe singleton: Admin Live URLs are SDK Host URLs (no path translation).
#[must_use]
pub fn overlay_fc_observe_direct_browser_urls(
    mut tap: ClawTapSettingsPublic,
    internal_live_base: &str,
    _fc_domain: &str,
) -> ClawTapSettingsPublic {
    let internal = internal_live_base.trim().trim_end_matches('/');
    if internal.is_empty() {
        return tap;
    }
    tap.live_base_url = Some(internal.to_string());
    tap.live_session_url_template = Some(live_session_viewer_url_template(internal));
    tap.live_browser_hosts_line = None;
    tap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_proxy_detects_e2b_traffic_host() {
        assert!(should_use_fc_observe_browser_proxy(
            "http://3000-sbx_abc.supone.top"
        ));
        assert!(!should_use_fc_observe_browser_proxy(
            "http://127.0.0.1:3000"
        ));
    }

    #[test]
    fn parse_identity_from_sdk_host() {
        let (port, sid) = parse_e2b_traffic_identity("http://3000-sbx_abc.supone.top").unwrap();
        assert_eq!(port, 3000);
        assert_eq!(sid, "sbx_abc");
    }

    #[test]
    fn overlay_observe_urls_use_host_domain_no_hosts() {
        let tap = overlay_fc_observe_direct_browser_urls(
            ClawTapSettingsPublic {
                mode: crate::gateway_claw_tap_settings::ClawTapMode::Local,
                host: String::new(),
                proxy_port: 8080,
                live_port: None,
                updated_at_ms: 0,
                configured: true,
                proxy_base_url: None,
                live_base_url: None,
                live_session_url_template: None,
                live_browser_hosts_line: None,
            },
            "http://3000-sbx_abc.supone.top",
            "supone.top",
        );
        assert_eq!(
            tap.live_base_url.as_deref(),
            Some("http://3000-sbx_abc.supone.top")
        );
        assert_eq!(
            tap.live_session_url_template.as_deref(),
            Some("http://3000-sbx_abc.supone.top/?session={sessionId}")
        );
        assert!(tap.live_browser_hosts_line.is_none());
    }
}
