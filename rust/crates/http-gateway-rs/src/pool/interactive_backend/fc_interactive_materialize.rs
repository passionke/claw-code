//! PG → e2b sandbox scripts: proj bake (warm pool) + session attach (acquire). Author: kejiqing

use std::collections::BTreeMap;

use base64::Engine;
use serde_json::json;

use crate::gateway_global_settings;
use crate::project_config_apply;
use crate::project_config_draft;
use crate::session_db::GatewaySessionDb;

pub(crate) const PROJ_HOME: &str = "/claw_ds";
pub(crate) const SESSION_HOME: &str = "/claw_host_root";
pub(crate) const OVS_WORKSPACE_ROOT: &str = "/claw_ws";

/// Self-hosted: mount NAS export root → `/claw_ws` (OVS singleton).
/// Firecracker sandboxes often lack `CAP_SYS_ADMIN`; exec mount may fail — fall back to local dir.
#[must_use]
pub fn self_hosted_nas_root_mount_sh(nas_server: &str, nas_export: &str) -> String {
    let export = nas_export.trim_end_matches('/');
    format!(
        r#"NAS_SERVER={nas_server:?}
NAS_EXPORT={export:?}
sudo -n mkdir -p {OVS_WORKSPACE_ROOT}
if ! mountpoint -q {OVS_WORKSPACE_ROOT} 2>/dev/null; then
  if ! sudo -n mount -t nfs4 "${{NAS_SERVER}}:${{NAS_EXPORT}}" {OVS_WORKSPACE_ROOT} -o vers=4.2,_netdev 2>/dev/null; then
    echo "warn: NAS mount ${{NAS_SERVER}}:${{NAS_EXPORT}} -> {OVS_WORKSPACE_ROOT} failed; using local dir" >&2
    sudo -n chown "$(id -u):$(id -g)" {OVS_WORKSPACE_ROOT}
  fi
fi
"#
    )
}

/// Start openvscode-server for the OVS singleton (NAS root at `{OVS_WORKSPACE_ROOT}`).
#[must_use]
pub fn start_ovs_server_sh(port: u16) -> String {
    format!(
        r#"set -e
OVS_BIN="/home/.openvscode-server/bin/openvscode-server"
if [ ! -x "$OVS_BIN" ]; then
  echo "openvscode-server not found in claw-ovs template" >&2
  exit 127
fi
if [ -f {OVS_WORKSPACE_ROOT}/.claw-ovs.pid ]; then
  kill "$(cat {OVS_WORKSPACE_ROOT}/.claw-ovs.pid)" 2>/dev/null || true
fi
export HOME=/opt/claw-ovs/home
mkdir -p /opt/claw-ovs/home /opt/claw-extensions /opt/claw-ovs/server-data/data/logs {OVS_WORKSPACE_ROOT}
nohup "$OVS_BIN" \
  --host=0.0.0.0 --port={port} \
  --without-connection-token \
  --server-base-path=/ovs \
  --default-folder={OVS_WORKSPACE_ROOT} \
  --extensions-dir=/opt/claw-extensions \
  --server-data-dir=/opt/claw-ovs/server-data \
  --enable-proposed-api=claw.claw-vscode,claw.ovs-chat-demo \
  >{OVS_WORKSPACE_ROOT}/.claw-ovs.log 2>&1 &
echo $! > {OVS_WORKSPACE_ROOT}/.claw-ovs.pid
sleep 1
curl -fsS "http://127.0.0.1:{port}/ovs/" >/dev/null
"#
    )
}

/// Self-hosted: mount `proj_N/home` → `/claw_ds` (warm bake; project-bound worker).
#[must_use]
pub fn self_hosted_proj_mount_sh(proj_id: i64, nas_server: &str, nas_export: &str) -> String {
    let export = nas_export.trim_end_matches('/');
    let proj_home_rel = format!("proj_{proj_id}/home");
    format!(
        r#"NAS_SERVER={nas_server:?}
NAS_EXPORT={export:?}
PROJ_HOME_REL={proj_home_rel:?}
sudo -n mkdir -p /claw_ds
if ! mountpoint -q /claw_ds 2>/dev/null; then
  if ! sudo -n mount -t nfs4 "${{NAS_SERVER}}:${{NAS_EXPORT}}/${{PROJ_HOME_REL}}" /claw_ds -o vers=4.2,_netdev 2>/dev/null; then
    echo "warn: NAS mount ${{NAS_SERVER}}:${{NAS_EXPORT}}/${{PROJ_HOME_REL}} -> /claw_ds failed; using local dir" >&2
    sudo -n chown "$(id -u):$(id -g)" /claw_ds
  fi
fi
"#
    )
}

/// Self-hosted: mount session tree → `/claw_host_root` (per acquire).
#[must_use]
pub fn self_hosted_session_mount_sh(
    session_id: &str,
    proj_id: i64,
    nas_server: &str,
    nas_export: &str,
) -> String {
    let export = nas_export.trim_end_matches('/');
    let session_rel = format!("proj_{proj_id}/sessions/{session_id}");
    format!(
        r#"NAS_SERVER={nas_server:?}
NAS_EXPORT={export:?}
if mountpoint -q /claw_host_root 2>/dev/null; then
  sudo -n umount /claw_host_root 2>/dev/null || true
fi
SESSION_REL={session_rel:?}
sudo -n mkdir -p /claw_host_root
if ! mountpoint -q /claw_host_root 2>/dev/null; then
  if ! sudo -n mount -t nfs4 "${{NAS_SERVER}}:${{NAS_EXPORT}}/${{SESSION_REL}}" /claw_host_root -o vers=4.2,_netdev 2>/dev/null; then
    echo "warn: NAS mount ${{NAS_SERVER}}:${{NAS_EXPORT}}/${{SESSION_REL}} -> /claw_host_root failed; using local dir" >&2
    sudo -n chown "$(id -u):$(id -g)" /claw_host_root
  fi
fi
mkdir -p /claw_host_root/.claw/sessions /claw_host_root/.config /claw_host_root/.cache /claw_host_root/.local/share
"#
    )
}

/// Return warm slot to idle: stop ttyd and drop session mount; keep `/claw_ds` baked.
#[must_use]
pub fn session_release_sh() -> &'static str {
    r#"set -e
if [ -f /claw_host_root/.claw/ttyd.pid ]; then
  kill "$(cat /claw_host_root/.claw/ttyd.pid)" 2>/dev/null || true
  rm -f /claw_host_root/.claw/ttyd.pid
fi
if mountpoint -q /claw_host_root 2>/dev/null; then
  umount /claw_host_root 2>/dev/null || true
fi
"#
}

/// Project config from PG → `/claw_ds` (warm pool bake; no session files).
pub async fn build_proj_bake_script(
    session_db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<String, String> {
    let mut lines = vec!["set -e".to_string()];
    let row = project_config_draft::row_for_materialize(session_db, proj_id)
        .await
        .map_err(|e| format!("load project_config: {e}"))?;
    if let Some(row) = row {
        let scaffold = gateway_global_settings::load_system_prompt_default(session_db)
            .await
            .map_err(|e| format!("load system prompt scaffold: {e}"))?;
        let writes = project_config_apply::build_guest_materialize_writes(&row, &scaffold)
            .map_err(|e| format!("build guest materialize writes: {e}"))?;
        for write in writes {
            let rel = write.rel_path.to_string_lossy();
            let path = format!("{PROJ_HOME}/{rel}");
            lines.push(shell_write_bytes(&path, &write.bytes));
        }
    } else {
        lines.push(format!("mkdir -p {PROJ_HOME}"));
    }
    let settings_bytes = serde_json::to_string_pretty(&json!({ "claw.projId": proj_id }))
        .map_err(|e| format!("serialize vscode settings: {e}"))?
        + "\n";
    lines.push(shell_write_bytes(
        &format!("{PROJ_HOME}/.vscode/settings.json"),
        settings_bytes.as_bytes(),
    ));
    Ok(lines.join("\n"))
}

/// Session-only files on `/claw_host_root` (LLM env); project already baked on worker.
pub fn build_session_attach_script(llm_env: &BTreeMap<String, String>) -> String {
    let mut lines = vec!["set -e".to_string()];
    lines.push(format!(
        "mkdir -p {SESSION_HOME}/.claw/sessions {SESSION_HOME}/.config {SESSION_HOME}/.cache {SESSION_HOME}/.local/share"
    ));
    lines.push(shell_write_bytes(
        &format!("{SESSION_HOME}/.claw/terminal-llm.env"),
        shell_export_env_file(llm_env).as_bytes(),
    ));
    lines.join("\n")
}

fn shell_write_bytes(abs_path: &str, bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(
        r#"mkdir -p "$(dirname "{abs_path}")" && printf '%s' '{b64}' | base64 -d > "{abs_path}""#
    )
}

/// Push guest files into FC sandbox via exec (self-hosted when gateway workspace ≠ NAS export).
#[must_use]
pub fn build_fc_guest_writes_script(root: &str, files: &[(String, Vec<u8>)]) -> String {
    let mut lines = vec!["set -e".to_string()];
    for (rel, bytes) in files {
        let abs = if rel.starts_with('/') {
            rel.clone()
        } else {
            format!("{root}/{rel}")
        };
        lines.push(shell_write_bytes(&abs, bytes));
    }
    lines.join("\n")
}

fn shell_export_env_file(env: &BTreeMap<String, String>) -> String {
    let mut out = String::from("# terminal worker LLM env (Admin active LLM + clawTap)\n");
    for (key, value) in env {
        out.push_str("export ");
        out.push_str(key);
        out.push('=');
        out.push_str(&shell_single_quote(value));
        out.push('\n');
    }
    out
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_write_bytes_marker() {
        let script = shell_write_bytes("/claw_ds/CLAUDE.md", b"# hello");
        assert!(script.contains("base64 -d"));
    }

    #[test]
    fn session_release_umounts_host_root() {
        assert!(session_release_sh().contains("umount /claw_host_root"));
    }

    #[test]
    fn proj_mount_targets_claw_ds_only() {
        let sh = self_hosted_proj_mount_sh(2, "10.8.0.8", "/export");
        assert!(sh.contains("/claw_ds"));
        assert!(sh.contains("proj_2/home"));
        assert!(!sh.contains("/claw_host_root"));
    }

    #[test]
    fn nas_root_mount_warns_on_failure() {
        let sh = self_hosted_nas_root_mount_sh("10.8.0.8", "/mnt/NAS0/nfs-export");
        assert!(sh.contains("warn: NAS mount"));
        assert!(sh.contains("using local dir"));
    }
}
