//! e2b sandbox guest materialize scripts (no in-guest NFS mount). Author: kejiqing
//!
//! NAS bind is static at sandbox create: `/claw_host_root` = `proj_N/workers/{workerId}`.
//! Gateway links `proj_N/sessions/{session}` → worker when the interactive lease is acquired.

use std::collections::BTreeMap;

use base64::Engine;
use claw_e2b_sandbox_client::{GUEST_CLAW_DS, GUEST_CLAW_HOST_ROOT};
use serde_json::json;

use crate::gateway_global_settings;
use crate::project_config_apply;
use crate::project_config_draft;
use crate::session_db::GatewaySessionDb;

pub(crate) const PROJ_HOME: &str = GUEST_CLAW_DS;
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
    fn session_attach_writes_llm_env_on_flat_work_root() {
        let mut env = BTreeMap::new();
        env.insert("CLAW_DEFAULT_MODEL".to_string(), "openai/mimo-v2.5".into());
        let sh = build_session_attach_script(&env);
        assert!(sh.contains("/claw_host_root/.claw/terminal-llm.env"));
        assert!(!sh.contains("ttyd"));
    }
}
