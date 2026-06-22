//! FC sandbox guest materialize scripts (no in-guest NFS mount). Author: kejiqing
//!
//! NAS bind is static at sandbox create: `/claw_host_root` = `proj_N/workers/{workerId}`.
//! Gateway links `proj_N/sessions/{session}` → worker at terminal/start.

use std::collections::BTreeMap;

use base64::Engine;
use claw_fc_sandbox_client::{GUEST_CLAW_DS, GUEST_CLAW_HOST_ROOT};
use serde_json::json;

use crate::gateway_global_settings;
use crate::project_config_apply;
use crate::project_config_draft;
use crate::session_db::GatewaySessionDb;

pub(crate) const PROJ_HOME: &str = GUEST_CLAW_DS;
pub(crate) const WORK_ROOT: &str = GUEST_CLAW_HOST_ROOT;
/// OVS / observe singleton NAS export root inside sandbox.
pub const OVS_WORKSPACE_ROOT: &str = claw_fc_sandbox_client::GUEST_CLAW_WS;

/// Start openvscode-server on OVS singleton (NAS root already at [`OVS_WORKSPACE_ROOT`]).
#[must_use]
pub fn start_ovs_server_sh(port: u16) -> String {
    format!(
        r#"set -e
OVS_BIN="/home/.openvscode-server/bin/openvscode-server"
if [ ! -x "$OVS_BIN" ]; then
  echo "fc ovs: openvscode-server not found (rebuild claw-ovs template)" >&2
  exit 127
fi
OVS_LOG="{OVS_WORKSPACE_ROOT}/.claw-ovs.log"
OVS_PID="{OVS_WORKSPACE_ROOT}/.claw-ovs.pid"
if [ -f "$OVS_PID" ] && kill -0 "$(cat "$OVS_PID")" 2>/dev/null; then
  if curl -fsS --connect-timeout 2 "http://127.0.0.1:{port}/ovs/" >/dev/null 2>&1; then
    exit 0
  fi
  kill "$(cat "$OVS_PID")" 2>/dev/null || true
  rm -f "$OVS_PID"
fi
export HOME=/opt/claw-ovs/home
mkdir -p /opt/claw-ovs/home /opt/claw-extensions /opt/claw-ovs/server-data/data/logs /opt/claw-ovs/server-data/data/Machine {OVS_WORKSPACE_ROOT}
nohup "$OVS_BIN" \
  --host=0.0.0.0 --port={port} \
  --without-connection-token \
  --server-base-path=/ovs \
  --extensions-dir=/opt/claw-extensions \
  --server-data-dir=/opt/claw-ovs/server-data \
  --enable-proposed-api=claw.claw-vscode,claw.ovs-chat-demo \
  >"$OVS_LOG" 2>&1 &
echo $! >"$OVS_PID"
for _ in $(seq 1 30); do
  if curl -fsS --connect-timeout 2 "http://127.0.0.1:{port}/ovs/" >/dev/null 2>&1; then
    exit 0
  fi
  sleep 1
done
echo "fc ovs: openvscode /ovs/ timeout (see $OVS_LOG)" >&2
exit 1
"#
    )
}

/// Start claude-tap Live on observe singleton (read NAS `tap-traces/` at [`OVS_WORKSPACE_ROOT`]).
/// Browser traffic uses E2B Host domain (`{port}-{sandboxId}.supone.top`); no path prefix env.
#[must_use]
pub fn start_observe_server_sh(live_port: u16, cluster_id: &str, db_url: &str) -> String {
    let tap_traces = format!("{OVS_WORKSPACE_ROOT}/tap-traces");
    format!(
        r#"set -e
OBS_LOG="{OVS_WORKSPACE_ROOT}/.claw-observe.log"
OBS_PID="{OVS_WORKSPACE_ROOT}/.claw-observe.pid"
TAP_BIN=""
for cand in /usr/local/bin/claude-tap /opt/claw-tap-runtime/bin/claude-tap; do
  if [ -x "$cand" ]; then TAP_BIN="$cand"; break; fi
done
if [ -z "$TAP_BIN" ]; then
  echo "fc observe: claude-tap not found (rebuild claw-observe template)" >&2
  exit 127
fi
if [ -f "$OBS_PID" ] && kill -0 "$(cat "$OBS_PID")" 2>/dev/null; then
  if curl -fsS --connect-timeout 2 "http://127.0.0.1:{live_port}/" >/dev/null 2>&1; then
    exit 0
  fi
fi
mkdir -p "{tap_traces}"
nohup env CLAW_CLUSTER_ID={cluster_id:?} CLAW_GATEWAY_DATABASE_URL={db_url:?} \
  "$TAP_BIN" \
  --tap-no-launch \
  --tap-live \
  --tap-host 0.0.0.0 \
  --tap-port 8080 \
  --tap-live-port {live_port} \
  --tap-target https://bootstrap.invalid/v1 \
  --tap-output-dir "{tap_traces}" \
  --tap-no-update-check \
  --tap-no-auto-update \
  >"$OBS_LOG" 2>&1 &
echo $! >"$OBS_PID"
for _ in $(seq 1 45); do
  if curl -fsS --connect-timeout 2 "http://127.0.0.1:{live_port}/" >/dev/null 2>&1; then
    exit 0
  fi
  sleep 1
done
echo "fc observe: Live / timeout (see $OBS_LOG)" >&2
exit 1
"#
    )
}

/// Stop ttyd on flat worker root (`/claw_host_root`).
#[must_use]
pub fn session_release_sh() -> String {
    format!(
        r#"set -e
if [ -f {WORK_ROOT}/.claw/ttyd.pid ]; then
  kill "$(cat {WORK_ROOT}/.claw/ttyd.pid)" 2>/dev/null || true
  rm -f {WORK_ROOT}/.claw/ttyd.pid
fi
"#
    )
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

/// Session files on flat `/claw_host_root`; project already baked on worker.
/// Per-prompt dialogue `record_session_id` is staged separately — see
/// `gateway-solve-turn::GATEWAY_RECORD_SESSION_ID_GUEST` and
/// `docs/ovs-chat/OVS-INTERACTIVE-SESSION-ID.md`. Author: kejiqing
pub fn build_session_attach_script(llm_env: &BTreeMap<String, String>) -> String {
    let mut lines = vec!["set -e".to_string()];
    lines.push(format!(
        "mkdir -p {WORK_ROOT}/.claw/sessions {WORK_ROOT}/.config {WORK_ROOT}/.cache {WORK_ROOT}/.local/share"
    ));
    lines.push(shell_write_bytes(
        &format!("{WORK_ROOT}/.claw/terminal-llm.env"),
        shell_export_env_file(llm_env).as_bytes(),
    ));
    lines.join("\n")
}

/// Start ttyd for interactive session; cwd = `/claw_ds` for OVS REPL else worker root.
#[must_use]
pub fn build_start_ttyd_script(session_id: &str) -> String {
    let ovs = session_id.starts_with("ovs-");
    let ttyd_cwd = if ovs { PROJ_HOME } else { WORK_ROOT };
    format!(
        r#"set -e
if ! command -v ttyd >/dev/null 2>&1; then
  echo 'ttyd not installed in worker image' >&2
  exit 127
fi
WORK={WORK_ROOT:?}
if [ -f "$WORK/.claw/ttyd.pid" ]; then
  kill "$(cat "$WORK/.claw/ttyd.pid")" 2>/dev/null || true
fi
export HOME="$WORK"
export CLAW_PROJECT_CONFIG_ROOT={PROJ_HOME:?}
export CLAW_GATEWAY_WORK_ROOT="$WORK"
export CLAW_DISPLAY_MODE=web
export XDG_CONFIG_HOME="$WORK/.config"
export XDG_CACHE_HOME="$WORK/.cache"
export XDG_DATA_HOME="$WORK/.local/share"
mkdir -p "$WORK/.claw/sessions" "$WORK/.config" "$WORK/.cache" "$WORK/.local/share"
if [ -f "$WORK/.claw/terminal-llm.env" ]; then
  set -a
  # shellcheck source=/dev/null
  . "$WORK/.claw/terminal-llm.env"
  set +a
fi
MODEL="${{CLAW_DEFAULT_MODEL:-openai/mimo-v2.5}}"
nohup ttyd -d 1 -i 0.0.0.0 -p 7681 -W -w {ttyd_cwd:?} \
  claw --allow-broad-cwd --model "$MODEL" \
  >"$WORK/.claw/ttyd.log" 2>&1 &
echo $! >"$WORK/.claw/ttyd.pid"
sleep 0.5
kill -0 "$(cat "$WORK/.claw/ttyd.pid")" 2>/dev/null
"#
    )
}

fn shell_write_bytes(abs_path: &str, bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!(
        r#"mkdir -p "$(dirname "{abs_path}")" && printf '%s' '{b64}' | base64 -d > "{abs_path}""#
    )
}

/// Push guest files into FC sandbox via exec.
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
    fn session_release_uses_flat_work_root() {
        let sh = session_release_sh();
        assert!(sh.contains("/claw_host_root/.claw/ttyd.pid"));
    }

    #[test]
    fn start_ttyd_uses_flat_work_root() {
        let sh = build_start_ttyd_script("sess-abc");
        assert!(sh.contains("/claw_host_root"));
        assert!(!sh.contains("/claw_host_root/sess-abc"));
    }

    #[test]
    fn ovs_start_script_uses_claw_ws() {
        let sh = start_ovs_server_sh(3000);
        assert!(sh.contains(OVS_WORKSPACE_ROOT));
    }

    #[test]
    fn observe_start_script_uses_tap_live_flags() {
        let sh = start_observe_server_sh(
            3000,
            "local-dev",
            "postgres://u:p@10.8.0.10:5433/claw_gateway",
        );
        assert!(sh.contains("--tap-no-launch"));
        assert!(sh.contains("--tap-live"));
        assert!(!sh.contains("CLAUDE_TAP_LIVE_PREFIX_PATH"));
        assert!(!sh.contains("claude-tap serve"));
        assert!(sh.contains("/claw_ws/tap-traces"));
    }
}
