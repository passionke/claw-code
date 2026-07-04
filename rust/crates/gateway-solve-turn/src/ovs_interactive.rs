//! OVS `@claw` interactive transcript paths and guest exec scripts (per `sessionId`).
//! Author: kejiqing
//!
//! Context SoT: `{clusterId}/proj_{N}/sessions/{segment}/.claw/interactive-session.jsonl`
//! (`/claw_sessions/{segment}` in worker). `worker_session_id` is lease-only.

use std::path::{Path, PathBuf};

use base64::Engine;

pub const GUEST_CLAW_DS: &str = "/claw_ds";
pub const GUEST_CLAW_SESSIONS: &str = "/claw_sessions";
pub const GUEST_CLAW_HOST_ROOT: &str = "/claw_host_root";

pub const OVS_INTERACTIVE_JSONL_NAME: &str = "interactive-session.jsonl";
pub const OVS_INTERACTIVE_REL: &str = ".claw/interactive-session.jsonl";

/// Guest path: `/claw_sessions/{segment}/.claw/interactive-session.jsonl`.
#[must_use]
pub fn ovs_interactive_jsonl_guest(segment: &str) -> String {
    format!("{GUEST_CLAW_SESSIONS}/{segment}/{OVS_INTERACTIVE_REL}")
}

/// Gateway host path under `work_root` (NAS bind).
#[must_use]
pub fn ovs_interactive_jsonl_host(
    work_root: &Path,
    cluster_id: &str,
    proj_id: i64,
    segment: &str,
) -> PathBuf {
    work_root
        .join(cluster_id)
        .join(format!("proj_{proj_id}"))
        .join("sessions")
        .join(segment)
        .join(OVS_INTERACTIVE_REL)
}

/// `session_id` field stored inside jsonl `session_meta` (file-local id).
#[must_use]
pub fn ovs_interactive_meta_session_id(segment: &str) -> String {
    format!("ovs-interactive-{segment}")
}

/// Idempotent: mkdir + bootstrap empty `session_meta` when jsonl missing. Author: kejiqing
#[must_use]
pub fn build_ensure_ovs_interactive_session_script(segment: &str) -> String {
    let jsonl = shell_single_quote(&ovs_interactive_jsonl_guest(segment));
    let meta_line = serde_json::json!({
        "type": "session_meta",
        "session_id": ovs_interactive_meta_session_id(segment),
        "version": 1,
        "created_at_ms": 0_i64,
        "updated_at_ms": 0_i64,
        "workspace_root": GUEST_CLAW_DS,
    })
    .to_string();
    let meta_b64 = base64::engine::general_purpose::STANDARD.encode(meta_line.as_bytes());
    format!(
        r#"set -e
JSONL={jsonl}
mkdir -p "$(dirname "$JSONL")"
if [ ! -f "$JSONL" ]; then
  printf '%s' '{meta_b64}' | base64 -d > "$JSONL"
else
  python3 - "$JSONL" <<'PY'
import json, sys
path = sys.argv[1]
want = "/claw_ds"
with open(path, "r+", encoding="utf-8") as f:
    first = f.readline()
    if not first.strip():
        sys.exit(0)
    meta = json.loads(first)
    if meta.get("type") != "session_meta":
        sys.exit(0)
    if meta.get("workspace_root") == want:
        sys.exit(0)
    meta["workspace_root"] = want
    rest = f.read()
    f.seek(0)
    f.write(json.dumps(meta, separators=(",", ":")) + "\n" + rest)
    f.truncate()
PY
fi"#
    )
}

/// One-shot OVS turn: `claw gateway-interactive-once` with web CDP on stdout. Author: kejiqing
#[must_use]
pub fn build_ovs_interactive_prompt_script(
    segment: &str,
    record_session_id: &str,
    prompt: &str,
    model: &str,
) -> String {
    let jsonl = shell_single_quote(&ovs_interactive_jsonl_guest(segment));
    let record_sid = shell_single_quote(record_session_id.trim());
    let session_root = shell_single_quote(&format!("{GUEST_CLAW_SESSIONS}/{segment}"));
    let proj_home = shell_single_quote(GUEST_CLAW_DS);
    let worker_root = shell_single_quote(GUEST_CLAW_HOST_ROOT);
    let model = shell_single_quote(
        model
            .trim()
            .is_empty()
            .then_some("openai/mimo-v2.5")
            .unwrap_or(model.trim()),
    );
    let prompt_b64 = base64::engine::general_purpose::STANDARD.encode(prompt.as_bytes());
    let ensure = build_ensure_ovs_interactive_session_script(segment);
    format!(
        r#"{ensure}
set -e
cd {proj_home}
export HOME={worker_root}
export CLAW_GATEWAY_WORK_ROOT={session_root}
export CLAW_PROJECT_CONFIG_ROOT={GUEST_CLAW_DS:?}
export CLAW_DISPLAY_MODE=web
export CLAW_SESSION_ID={record_sid}
if [ -f {GUEST_CLAW_HOST_ROOT:?}/.claw/terminal-llm.env ]; then
  set -a
  # shellcheck source=/dev/null
  . {GUEST_CLAW_HOST_ROOT:?}/.claw/terminal-llm.env
  set +a
fi
claw gateway-interactive-once \
  --session-jsonl {jsonl} \
  --prompt-b64 '{prompt_b64}' \
  --model {model} \
  --permission-mode danger-full-access"#
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_jsonl_path_uses_claw_sessions() {
        let p = ovs_interactive_jsonl_guest("ovs-chat-1-abc");
        assert!(p.starts_with("/claw_sessions/ovs-chat-1-abc/"));
        assert!(p.ends_with("interactive-session.jsonl"));
    }

    #[test]
    fn ensure_script_bootstraps_meta() {
        let sh = build_ensure_ovs_interactive_session_script("seg-a");
        assert!(sh.contains("interactive-session.jsonl"));
        assert!(sh.contains("base64 -d"));
        assert!(sh.contains("/claw_sessions/seg-a"));
        assert!(sh.contains(r#"want = "/claw_ds""#));
    }

    #[test]
    fn prompt_script_uses_gateway_interactive_once() {
        let sh = build_ovs_interactive_prompt_script(
            "seg-b",
            "ovs-chat-2-x",
            "hello",
            "openai/mimo-v2.5",
        );
        assert!(sh.contains("gateway-interactive-once"));
        assert!(sh.contains("--prompt-b64"));
        assert!(sh.contains("CLAW_SESSION_ID='ovs-chat-2-x'"));
        assert!(sh.contains("cd '/claw_ds'"));
        assert!(sh.contains("export HOME='/claw_host_root'"));
        assert!(sh.contains("CLAW_GATEWAY_WORK_ROOT='/claw_sessions/seg-b'"));
        assert!(sh.contains("--model 'openai/mimo-v2.5'"));
        assert!(!sh.contains("--allow-broad-cwd"));
        assert!(!sh.contains("hello"));
    }

    #[test]
    fn prompt_script_falls_back_when_model_blank() {
        let sh = build_ovs_interactive_prompt_script("seg-c", "ovs-chat-1-x", "hi", "  ");
        assert!(sh.contains("--model 'openai/mimo-v2.5'"));
    }

    #[test]
    fn distinct_record_sessions_use_distinct_jsonl_paths() {
        let root = std::path::Path::new("/tmp/work");
        let a = ovs_interactive_jsonl_host(root, "dev", 3, "ovs-chat-3-a");
        let b = ovs_interactive_jsonl_host(root, "dev", 3, "ovs-chat-3-b");
        assert_ne!(a, b);
        assert!(a.to_string_lossy().contains("sessions/ovs-chat-3-a"));
        assert!(b.to_string_lossy().contains("sessions/ovs-chat-3-b"));
    }
}
