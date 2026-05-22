//! Resolve worker report SSE host/port after `podman|docker run` (gateway-reachable). Author: kejiqing

use super::docker_cli::runtime_inspect_container_ip;

/// Default host port for slot 0 (`CLAW_*_WORKER_REPORT_PUBLISH_BASE` fallback).
pub const DEFAULT_WORKER_REPORT_PUBLISH_BASE: u16 = 29_000;

/// How gateways should reach `GET :port/v1/turns/{id}/report` on the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerReportResolve {
    /// Container name on `CLAW_*_NETWORK` (DNS inside the network).
    ContainerName,
    /// Container IPv4 on that network (preferred for multi-gateway / explicit routing).
    ContainerIp,
    /// Map `publish_base + slot_index` on host → in-container SSE port.
    HostPublish,
}

impl WorkerReportResolve {
    /// `CLAW_PODMAN_WORKER_REPORT_RESOLVE` / `CLAW_DOCKER_WORKER_REPORT_RESOLVE`.
    /// When unset, defaults to [`HostPublish`] (daemon maps host port → in-container SSE). Author: kejiqing
    pub fn from_env(prefix: &str, _pool_network_configured: bool) -> Self {
        let key = format!("{prefix}WORKER_REPORT_RESOLVE");
        match std::env::var(&key)
            .ok()
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("container_name" | "name") => Self::ContainerName,
            Some("container_ip" | "ip") => Self::ContainerIp,
            Some("host_publish" | "publish") => Self::HostPublish,
            Some(other) => {
                tracing::warn!(
                    target: "claw_gateway_pool",
                    component = "worker_report_endpoint",
                    env = %key,
                    value = %other,
                    "unknown WORKER_REPORT_RESOLVE; using default"
                );
                default_resolve()
            }
            None => default_resolve(),
        }
    }
}

fn default_resolve() -> WorkerReportResolve {
    WorkerReportResolve::HostPublish
}

/// Published host port for a pool slot (`base + slot_index`). Author: kejiqing
#[must_use]
pub fn host_publish_port(publish_base: u16, slot_index: usize) -> Option<u16> {
    let sum = u32::from(publish_base) + u32::try_from(slot_index).ok()?;
    u16::try_from(sum).ok()
}

/// `podman run -p` mapping: listen on all host interfaces → in-container SSE port. Author: kejiqing
#[must_use]
pub fn host_publish_port_mapping(
    publish_base: u16,
    slot_index: usize,
    container_sse_port: u16,
) -> Option<String> {
    let host_port = host_publish_port(publish_base, slot_index)?;
    Some(format!("0.0.0.0:{host_port}:{container_sse_port}"))
}

/// Pick `(host, port)` stored on the lease and in `gateway_turns`. Author: kejiqing
#[allow(clippy::too_many_arguments)]
pub async fn resolve_worker_report_endpoint(
    runtime_bin: &str,
    mode: WorkerReportResolve,
    pool_network: Option<&str>,
    container_name: &str,
    slot_index: usize,
    container_sse_port: u16,
    advertise_host: &str,
    publish_base: Option<u16>,
) -> (String, u16) {
    match mode {
        WorkerReportResolve::ContainerName => (container_name.to_string(), container_sse_port),
        WorkerReportResolve::ContainerIp => {
            if let Some(net) = pool_network {
                if let Some(ip) =
                    runtime_inspect_container_ip(runtime_bin, container_name, net).await
                {
                    return (ip, container_sse_port);
                }
                tracing::warn!(
                    target: "claw_gateway_pool",
                    component = "worker_report_endpoint",
                    container = %container_name,
                    network = %net,
                    "inspect IP failed; falling back to container name"
                );
            }
            (container_name.to_string(), container_sse_port)
        }
        WorkerReportResolve::HostPublish => {
            let port = publish_base
                .and_then(|b| host_publish_port(b, slot_index))
                .unwrap_or(container_sse_port);
            let host = advertise_host.trim();
            let host = if host.is_empty() { "127.0.0.1" } else { host };
            (host.to_string(), port)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_resolve_is_host_publish() {
        assert_eq!(default_resolve(), WorkerReportResolve::HostPublish);
    }

    #[test]
    fn host_publish_port_mapping_binds_all_interfaces() {
        assert_eq!(
            host_publish_port_mapping(29_000, 1, 18765).as_deref(),
            Some("0.0.0.0:29001:18765")
        );
    }

    #[test]
    fn host_publish_port_offsets_by_slot() {
        assert_eq!(host_publish_port(29_000, 0), Some(29_000));
        assert_eq!(host_publish_port(29_000, 3), Some(29_003));
        assert!(host_publish_port(65_000, 1000).is_none());
    }
}
