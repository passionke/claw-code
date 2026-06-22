//! ttyd WebSocket URL helpers for podman vs FC backends. Author: kejiqing

/// Where the gateway dials ttyd (loopback podman or FC public host).
#[derive(Debug, Clone)]
pub struct TtydConnectTarget {
    pub host: String,
    pub port: u16,
    pub use_tls: bool,
    /// Self-hosted e2b: dial `host:port` but set HTTP Host to this subdomain.
    pub proxy_host_header: Option<String>,
    /// e2b traffic proxy token (`X-Access-Token` or `?token=`).
    pub traffic_access_token: Option<String>,
}

impl TtydConnectTarget {
    #[must_use]
    pub fn loopback(port: u16, default_host: &str) -> Self {
        Self {
            host: default_host.to_string(),
            port,
            use_tls: false,
            proxy_host_header: None,
            traffic_access_token: None,
        }
    }

    #[must_use]
    pub fn fc_public(host: String) -> Self {
        Self {
            host,
            port: 443,
            use_tls: true,
            proxy_host_header: None,
            traffic_access_token: None,
        }
    }

    /// Self-hosted e2bserver: connect to traffic proxy (`10.8.0.9:80`) with Host `{port}-{id}.{domain}`.
    /// (`CLAW_E2B_SANDBOX_URL` :3002 is envd/exec only.)
    #[must_use]
    pub fn e2b_self_hosted_proxy(
        proxy_host: String,
        proxy_port: u16,
        proxy_host_header: String,
        traffic_access_token: Option<String>,
    ) -> Self {
        Self {
            host: proxy_host,
            port: proxy_port,
            use_tls: false,
            proxy_host_header: Some(proxy_host_header),
            traffic_access_token,
        }
    }
}

#[must_use]
pub fn terminal_ws_connect_url(target: &TtydConnectTarget) -> String {
    let scheme = if target.use_tls { "wss" } else { "ws" };
    let base = if target.use_tls && target.port == 443 {
        format!("{scheme}://{}/ws", target.host)
    } else {
        format!("{scheme}://{}:{}/ws", target.host, target.port)
    };
    claw_fc_sandbox_client::FcSandboxClient::traffic_url(
        &base,
        target.traffic_access_token.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fc_wss_url_uses_host_prefix_port() {
        let url = terminal_ws_connect_url(&TtydConnectTarget::fc_public(
            "7681-sbx-1.cn-beijing.e2b.fc.aliyuncs.com".into(),
        ));
        assert_eq!(url, "wss://7681-sbx-1.cn-beijing.e2b.fc.aliyuncs.com/ws");
    }

    #[test]
    fn podman_ws_url() {
        let url = terminal_ws_connect_url(&TtydConnectTarget::loopback(37681, "127.0.0.1"));
        assert_eq!(url, "ws://127.0.0.1:37681/ws");
    }
}
