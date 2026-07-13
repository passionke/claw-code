//! e2b sandbox guest materialize scripts (no in-guest NFS mount). Author: kejiqing
//!
//! NAS bind is static at sandbox create: `/claw_host_root` = `proj_N/workers/{workerId}`.
//! Gateway links `proj_N/sessions/{session}` → worker at terminal/start.

use std::collections::BTreeMap;

use base64::Engine;
use claw_e2b_sandbox_client::{GUEST_CLAW_DS, GUEST_CLAW_HOST_ROOT};
use serde_json::json;

use crate::gateway_global_settings;
use crate::project_config_apply;
use crate::project_config_draft;
use crate::session_db::GatewaySessionDb;

pub(crate) const PROJ_HOME: &str = GUEST_CLAW_DS;
/// Stable project config root on NAS (`home/project_home_def` → versioned tree). Author: kejiqing
pub(crate) const PROJ_CONFIG_ROOT: &str = "/claw_ds/project_home_def";
pub(crate) const WORK_ROOT: &str = GUEST_CLAW_HOST_ROOT;

/// Project config from PG → `/claw_ds` (proj worker bind; no session files).
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
export CLAW_PROJECT_CONFIG_ROOT={PROJ_CONFIG_ROOT:?}
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

/// Push guest files into e2b sandbox via exec.
#[must_use]
pub fn build_e2b_guest_writes_script(root: &str, files: &[(String, Vec<u8>)]) -> String {
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
    fn start_ttyd_uses_flat_work_root() {
        let sh = build_start_ttyd_script("sess-abc");
        assert!(sh.contains("/claw_host_root"));
        assert!(!sh.contains("/claw_host_root/sess-abc"));
    }
}
