//! FC cloud sandbox interactive backend. Author: kejiqing

use std::sync::Arc;

use claw_fc_sandbox_client::FcSandboxClient;

use super::{
    InteractiveBackendKind, InteractiveLease, InteractiveSandboxBackend, InteractiveSessionSpec,
    TtydConnectTarget,
};

pub struct FcInteractiveBackend {
    client: Arc<FcSandboxClient>,
    pool_id: String,
}

impl FcInteractiveBackend {
    #[must_use]
    pub fn new(client: Arc<FcSandboxClient>, pool_id: String) -> Self {
        Self { client, pool_id }
    }

    fn ttyd_target(&self, handle: &claw_fc_sandbox_client::FcSandboxHandle) -> TtydConnectTarget {
        if handle.ttyd_use_tls {
            return TtydConnectTarget::fc_public(handle.ttyd_public_host.clone());
        }
        let cfg = self.client.config();
        let proxy = cfg
            .sandbox_url
            .as_deref()
            .unwrap_or("http://127.0.0.1:3002");
        let (host, port) = parse_proxy_base(proxy);
        TtydConnectTarget::e2b_self_hosted_proxy(host, port, handle.ttyd_public_host.clone())
    }

    fn start_script(&self, spec: &InteractiveSessionSpec) -> String {
        let cfg = self.client.config();
        if !cfg.is_self_hosted() {
            return spec.start_ttyd_script.to_string();
        }
        let mount = self_hosted_nfs_mount_sh(
            &spec.session_id,
            spec.proj_id,
            spec.ovs_mode,
            cfg.nas_server.as_deref().unwrap_or("10.8.0.8"),
            cfg.nas_export.as_deref().unwrap_or("/"),
        );
        format!("{mount}\n{}", spec.start_ttyd_script)
    }
}

fn parse_proxy_base(url: &str) -> (String, u16) {
    let trimmed = url.trim().trim_end_matches('/');
    let no_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    if let Some((host, port)) = no_scheme.rsplit_once(':') {
        if let Ok(p) = port.parse::<u16>() {
            return (host.to_string(), p);
        }
    }
    (no_scheme.to_string(), 3002)
}

fn self_hosted_nfs_mount_sh(
    session_id: &str,
    proj_id: i64,
    ovs_mode: bool,
    nas_server: &str,
    nas_export: &str,
) -> String {
    let export = nas_export.trim_end_matches('/');
    let session_rel = format!("proj_{proj_id}/sessions/{session_id}");
    let proj_home_rel = format!("proj_{proj_id}/home");
    let ovs = if ovs_mode { "1" } else { "0" };
    format!(
        r#"set -e
NAS_SERVER={nas_server:?}
NAS_EXPORT={export:?}
mkdir -p /claw_host_root /claw_ds
if ! mountpoint -q /claw_host_root 2>/dev/null; then
  mount -t nfs4 "${{NAS_SERVER}}:${{NAS_EXPORT}}/{session_rel}" /claw_host_root -o vers=4.2,_netdev,nfsvers=4.2
fi
if [ {ovs:?} = "1" ] && ! mountpoint -q /claw_ds 2>/dev/null; then
  mount -t nfs4 "${{NAS_SERVER}}:${{NAS_EXPORT}}/{proj_home_rel}" /claw_ds -o vers=4.2,_netdev,nfsvers=4.2
fi
mkdir -p /claw_host_root/.claw/sessions /claw_host_root/.config /claw_host_root/.cache /claw_host_root/.local/share
"#
    )
}

#[async_trait::async_trait]
impl InteractiveSandboxBackend for FcInteractiveBackend {
    async fn start_session(
        &self,
        spec: InteractiveSessionSpec,
    ) -> Result<InteractiveLease, String> {
        let handle = self
            .client
            .create_sandbox(&spec.session_id, spec.proj_id, spec.ovs_mode)
            .await?;

        let script = self.start_script(&spec);
        if let Err(e) = self.client.exec_shell_script(&handle, &script).await {
            let _ = self.client.kill_sandbox(&handle.sandbox_id).await;
            return Err(format!("fc start ttyd: {e}"));
        }

        Ok(InteractiveLease {
            backend: InteractiveBackendKind::Fc,
            slot_index: 0,
            worker_name: Some(format!("fc:{}", handle.sandbox_id)),
            pool_id: self.pool_id.clone(),
            fc_sandbox_id: Some(handle.sandbox_id.clone()),
            ttyd: self.ttyd_target(&handle),
        })
    }

    async fn stop_session(&self, lease: &InteractiveLease) -> Result<(), String> {
        if lease.backend != InteractiveBackendKind::Fc {
            return Err("fc stop called on non-fc lease".into());
        }
        let sandbox_id = lease
            .fc_sandbox_id
            .as_deref()
            .ok_or_else(|| "fc lease missing sandbox_id".to_string())?;
        self.client.kill_sandbox(sandbox_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_proxy_base_host_port() {
        assert_eq!(
            parse_proxy_base("http://10.8.0.9:3002"),
            ("10.8.0.9".into(), 3002)
        );
    }
}
