//! Gateway-orchestrated `claw-vscode` on FC OVS singleton (after openvscode is up).
//! Reuses the same VSIX + Machine settings contract as `deploy/fc-sandbox/fc-ovs-install-claw-vscode.py`
//! — no parallel session model. Author: kejiqing
//!
//! See `docs/ovs-chat/EXTENSION-STABLE-DEPLOY.md`.

use std::path::Path;

use base64::Engine;
use claw_fc_sandbox_client::{FcSandboxClient, FcSandboxHandle};
use serde_json::{json, Value};
use tracing::{info, warn};

use super::fc_interactive_materialize::{start_ovs_server_sh, OVS_WORKSPACE_ROOT};

/// Packaged extension version (keep in sync with `extensions/claw-vscode/package.json`).
pub const DEFAULT_CLAW_VSCODE_VERSION: &str = "0.2.9";

/// Host:port the OVS sandbox uses for Remote EH → gateway `agent/ws` (not loopback on Mac).
#[must_use]
pub fn resolve_fc_ovs_gateway_host(gateway_port: u16) -> String {
    if let Ok(raw) = std::env::var("CLAW_FC_OVS_GATEWAY_HOST") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return if trimmed.contains(':') {
                trimmed.to_string()
            } else {
                format!("{trimmed}:{gateway_port}")
            };
        }
    }
    for key in ["CLAW_POOL_ADVERTISE_HOST", "CLAW_FC_GATEWAY_ADVERTISE_HOST"] {
        if let Ok(raw) = std::env::var(key) {
            let host = raw.trim();
            if !host.is_empty() {
                return format!("{host}:{gateway_port}");
            }
        }
    }
    if let Ok(base) = std::env::var("CLAW_GATEWAY_BASE_URL") {
        if let Some(host) = parse_url_host(&base) {
            if !matches!(
                host.as_str(),
                "127.0.0.1" | "localhost" | "host.docker.internal"
            ) {
                return format!("{host}:{gateway_port}");
            }
        }
    }
    for key in ["CLAW_FC_WORKER_DATABASE_URL", "CLAW_GATEWAY_DATABASE_URL"] {
        if let Ok(db) = std::env::var(key) {
            if let Some(host) = postgres_host_from_url(&db) {
                if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "postgres") {
                    return format!("{host}:{gateway_port}");
                }
            }
        }
    }
    format!("10.8.0.2:{gateway_port}")
}

#[must_use]
pub fn resolve_fc_ovs_gateway_public_host(gateway_port: u16) -> String {
    std::env::var("CLAW_GATEWAY_PUBLIC_HOST")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| format!("127.0.0.1:{gateway_port}"))
}

fn parse_url_host(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))
        .unwrap_or(trimmed);
    let host = rest.split('/').next()?.split(':').next()?.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn postgres_host_from_url(db_url: &str) -> Option<String> {
    let after_at = db_url.rsplit('@').next()?;
    let hostport = after_at.split('/').next()?.trim();
    if hostport.is_empty() {
        return None;
    }
    Some(
        hostport
            .rsplit_once(':')
            .map(|(h, _)| h.to_string())
            .unwrap_or_else(|| hostport.to_string()),
    )
}

#[must_use]
pub fn fc_ovs_machine_settings(gateway_host: &str, gateway_public_host: &str) -> Value {
    json!({
        "files.watcherUsePolling": true,
        "chat.disableAIFeatures": false,
        "chat.agent.enabled": true,
        "chat.experimental.serverlessWebEnabled": false,
        "chat.experimental.disableCoreAgents": true,
        "chat.newSession.defaultMode": "ask",
        "claw.gatewayHost": gateway_host,
        "claw.gatewayPublicHost": gateway_public_host,
        "claw.playgroundPort": "18765",
        "extensions.autoUpdate": false,
        "extensions.autoCheckUpdates": false,
        "update.mode": "none",
        "extensions.gallery.serviceUrl": "https://disabled.invalid",
        "extensions.gallery.itemUrl": "https://disabled.invalid",
        "extensions.gallery.controlUrl": "https://disabled.invalid",
        "security.workspace.trust.enabled": false,
        "security.workspace.trust.startupPrompt": "never",
        "security.workspace.trust.banner": "never",
        "security.workspace.trust.emptyWindow": true,
        "security.workspace.trust.trustedFolders": [
            "/claw_ws",
            "/claw_ws/proj_1/home",
            "/claw_ws/proj_2/home",
            "/claw_ws/proj_3/home"
        ]
    })
}

/// Bundled VSIX in gateway image (`build.sh` packages before image build). Author: kejiqing
pub const BUNDLED_VSIX_DIR: &str = "/app/deploy/stack";

/// Load VSIX bytes: `CLAW_FC_OVS_VSIX` → NAS → gateway image bundle.
pub async fn load_claw_vscode_vsix(nas_root: &Path) -> Result<(Vec<u8>, String), String> {
    let ext_ver = claw_vscode_extension_version();

    if let Ok(raw) = std::env::var("CLAW_FC_OVS_VSIX") {
        let path = raw.trim();
        if !path.is_empty() {
            return read_vsix_file(path, &ext_ver).await;
        }
    }

    let tools_rel = std::env::var("CLAW_FC_NAS_TOOLS_REL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| ".claw-fc-tools".to_string());
    let nas_vsix = nas_root.join(&tools_rel).join("claw-vscode.vsix");
    if let Ok(out) = read_vsix_file(&nas_vsix, &ext_ver).await {
        return Ok(out);
    }

    let bundled = Path::new(BUNDLED_VSIX_DIR).join(format!("claw.claw-vscode-{ext_ver}.vsix"));
    if let Ok(out) = read_vsix_file(&bundled, &ext_ver).await {
        return Ok(out);
    }

    if let Ok(repo) = std::env::var("CLAW_REPO_ROOT") {
        let repo_vsix =
            Path::new(repo.trim()).join(format!("deploy/stack/claw.claw-vscode-{ext_ver}.vsix"));
        if let Ok(out) = read_vsix_file(&repo_vsix, &ext_ver).await {
            return Ok(out);
        }
    }

    Err(format!(
        "claw-vscode VSIX not found (set CLAW_FC_OVS_VSIX, NAS {}, or rebuild gateway image with packaged VSIX)",
        nas_vsix.display()
    ))
}

fn claw_vscode_extension_version() -> String {
    std::env::var("CLAW_VSCODE_EXTENSION_VERSION")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_CLAW_VSCODE_VERSION.to_string())
}

async fn read_vsix_file(
    path: impl AsRef<Path>,
    ext_ver: &str,
) -> Result<(Vec<u8>, String), String> {
    let path = path.as_ref();
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("read VSIX {}: {e}", path.display()))?;
    if bytes.len() < 1024 {
        return Err(format!(
            "VSIX too small ({} bytes): {}",
            bytes.len(),
            path.display()
        ));
    }
    Ok((bytes, ext_ver.to_string()))
}

/// Idempotent guest script: install VSIX + Machine settings; restart OVS when extension was missing.
#[must_use]
pub fn build_ensure_claw_vscode_script(
    ovs_port: u16,
    ext_ver: &str,
    vsix_b64: &str,
    machine_b64: &str,
    gateway_host: &str,
) -> String {
    let marker = format!("{OVS_WORKSPACE_ROOT}/.claw-ovs-claw-vscode-{ext_ver}");
    let tmp_vsix = format!("/tmp/claw-vscode-{ext_ver}.vsix");
    let restart = start_ovs_server_sh(ovs_port);
    format!(
        r#"set -e
OVS_BIN="/home/.openvscode-server/bin/openvscode-server"
EXT_DIR=/opt/claw-extensions
SD=/opt/claw-ovs/server-data
OVS_HOME=/opt/claw-ovs/home
VSIX="{tmp_vsix}"
MARKER="{marker}"
PORT={ovs_port}
EXT_VER={ext_ver:?}

if [ ! -x "$OVS_BIN" ]; then
  echo "fc ovs claw-vscode: openvscode-server missing (claw-ovs template)" >&2
  exit 127
fi

export HOME="$OVS_HOME"
mkdir -p "$OVS_HOME" "$EXT_DIR" "$SD/data/logs" "$SD/data/Machine" {OVS_WORKSPACE_ROOT}

MACHINE="$SD/Machine/settings.json"
MACHINE_DATA="$SD/data/Machine/settings.json"
mkdir -p "$(dirname "$MACHINE")" "$(dirname "$MACHINE_DATA")"
printf '%s' '{machine_b64}' | base64 -d >"$MACHINE"
cp -f "$MACHINE" "$MACHINE_DATA"

if [ -f "$MARKER" ] && [ -f "$EXT_DIR/claw.claw-vscode-$EXT_VER/extension.js" ]; then
  echo "fc ovs claw-vscode: already installed ($EXT_VER)"
  curl -fsS -m 8 "http://{gateway_host}/healthz" >/dev/null \
    || echo "fc ovs claw-vscode: warn gateway http://{gateway_host}/healthz not reachable from sandbox" >&2
  exit 0
fi

printf '%s' '{vsix_b64}' | base64 -d >"$VSIX"
if [ ! -s "$VSIX" ]; then
  echo "fc ovs claw-vscode: decoded VSIX empty" >&2
  exit 1
fi

if "$OVS_BIN" --list-extensions --extensions-dir="$EXT_DIR" --server-data-dir="$SD" 2>/dev/null | grep -q '^claw\.ovs-chat-demo$'; then
  "$OVS_BIN" --uninstall-extension claw.ovs-chat-demo \
    --extensions-dir="$EXT_DIR" --server-data-dir="$SD" 2>/dev/null || true
fi

echo "==> install-extension $VSIX"
"$OVS_BIN" --install-extension "$VSIX" \
  --extensions-dir="$EXT_DIR" \
  --server-data-dir="$SD" \
  --force

test -f "$EXT_DIR/claw.claw-vscode-$EXT_VER/extension.js" \
  || {{ echo "claw.claw-vscode extension.js missing after install" >&2; exit 1; }}

/home/.openvscode-server/node --check "$EXT_DIR/claw.claw-vscode-$EXT_VER/extension.js"

curl -fsS -m 8 "http://{gateway_host}/healthz" >/dev/null \
  || {{ echo "fc ovs claw-vscode: gateway http://{gateway_host}/healthz not reachable from sandbox" >&2; exit 1; }}

touch "$MARKER"
{restart}
"#
    )
}

/// Probe: extension files on disk (avoid `list-extensions` while OVS holds spdlog lock).
pub async fn probe_claw_vscode_installed(
    client: &FcSandboxClient,
    handle: &FcSandboxHandle,
) -> Result<(), String> {
    let ext_ver = claw_vscode_extension_version();
    let script = format!(r#"test -f /opt/claw-extensions/claw.claw-vscode-{ext_ver}/extension.js"#);
    client.exec_shell_script(handle, &script).await
}

/// After OVS HTTP is up: install/refresh claw-vscode (idempotent per extension version).
pub async fn ensure_claw_vscode_on_ovs(
    client: &FcSandboxClient,
    handle: &FcSandboxHandle,
    ovs_port: u16,
    nas_root: &Path,
    gateway_port: u16,
) -> Result<(), String> {
    let (vsix_bytes, ext_ver) = load_claw_vscode_vsix(nas_root).await?;
    let gateway_host = resolve_fc_ovs_gateway_host(gateway_port);
    let gateway_public = resolve_fc_ovs_gateway_public_host(gateway_port);
    let machine = fc_ovs_machine_settings(&gateway_host, &gateway_public);
    let machine_b64 = base64::engine::general_purpose::STANDARD.encode(
        serde_json::to_string_pretty(&machine)
            .map_err(|e| format!("serialize ovs machine settings: {e}"))?
            .as_bytes(),
    );
    let vsix_b64 = base64::engine::general_purpose::STANDARD.encode(&vsix_bytes);
    let script =
        build_ensure_claw_vscode_script(ovs_port, &ext_ver, &vsix_b64, &machine_b64, &gateway_host);
    client.exec_shell_script(handle, &script).await?;
    info!(
        target: "claw_fc_ovs",
        sandbox_id = %handle.sandbox_id,
        ext_version = %ext_ver,
        gateway_host = %gateway_host,
        vsix_bytes = vsix_bytes.len(),
        "claw-vscode ensured on ovs singleton"
    );
    Ok(())
}

/// Extension missing but OVS up — log and attempt install without failing caller when VSIX absent.
pub async fn ensure_claw_vscode_if_needed(
    client: &FcSandboxClient,
    handle: &FcSandboxHandle,
    ovs_port: u16,
    nas_root: &Path,
    gateway_port: u16,
) {
    if probe_claw_vscode_installed(client, handle).await.is_ok() {
        return;
    }
    match ensure_claw_vscode_on_ovs(client, handle, ovs_port, nas_root, gateway_port).await {
        Ok(()) => {}
        Err(e) => {
            warn!(
                target: "claw_fc_ovs",
                sandbox_id = %handle.sandbox_id,
                error = %e,
                "claw-vscode ensure failed (manual: deploy/stack/lib/install-claw-vscode-fc-ovs.sh)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_settings_enable_agent_chat() {
        let cfg = fc_ovs_machine_settings("10.8.0.2:8088", "127.0.0.1:8088");
        assert_eq!(cfg["chat.agent.enabled"], json!(true));
        assert_eq!(cfg["claw.gatewayHost"], json!("10.8.0.2:8088"));
    }

    #[test]
    fn ensure_script_mentions_claw_vscode_marker() {
        let sh = build_ensure_claw_vscode_script(3000, "0.2.9", "YQ==", "e30=", "10.8.0.2:8088");
        assert!(sh.contains("claw.claw-vscode"));
        assert!(sh.contains(".claw-ovs-claw-vscode-0.2.9"));
        assert!(sh.contains("install-extension"));
    }

    #[test]
    fn resolve_gateway_host_prefers_fc_ovs_env() {
        let prev = std::env::var("CLAW_FC_OVS_GATEWAY_HOST").ok();
        std::env::set_var("CLAW_FC_OVS_GATEWAY_HOST", "10.8.0.2");
        assert_eq!(resolve_fc_ovs_gateway_host(8088), "10.8.0.2:8088");
        match prev {
            Some(v) => std::env::set_var("CLAW_FC_OVS_GATEWAY_HOST", v),
            None => std::env::remove_var("CLAW_FC_OVS_GATEWAY_HOST"),
        }
    }
}
