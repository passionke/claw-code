//! OVS interactive `@claw` agent: fixed resume jsonl path + guest shell scripts. Author: kejiqing
//!
//! One Admin `record_session_id` → one NAS folder `proj_N/sessions/{segment}/` with a single
//! growing `interactive-session.jsonl`. Guest exec reads/writes via
//! `/claw_ds/.claw/interactive/{segment}/` symlink → `../../../sessions/{segment}` (Podman:
//! `/claw_ds` = full `proj_N`). FC warm worker also bind-mounts `proj_N/sessions` → `/claw_sessions`.

use std::path::{Path, PathBuf};

use base64::Engine;

/// Guest mount for `proj_N/sessions` (FC warm worker; Podman uses `/claw_ds/sessions` via full proj bind).
pub const OVS_INTERACTIVE_GUEST_SESSIONS_MOUNT: &str = "/claw_sessions";

/// Guest-relative prefix under `/claw_ds` (symlink into `proj_N/sessions/{segment}`).
pub const OVS_INTERACTIVE_GUEST_REL_PREFIX: &str = ".claw/interactive";

/// Legacy guest path (pre directory merge); used only for one-time migration in exec scripts.
pub const OVS_INTERACTIVE_LEGACY_GUEST_REL_PREFIX: &str = ".claw/ovs-chat";

/// Single transcript file per `record_session_id` segment.
pub const OVS_INTERACTIVE_SESSION_FILENAME: &str = "interactive-session.jsonl";

/// In-container project home mount (shared across interactive workers).
pub const OVS_INTERACTIVE_PROJ_HOME: &str = "/claw_ds";

/// Worker session root (`HOME` for claw harness side paths).
pub const OVS_INTERACTIVE_WORK_ROOT: &str = "/claw_host_root";

/// Canonical NAS host dir: `proj_{N}/sessions/{segment}`.
#[must_use]
pub fn ovs_interactive_session_dir_host(nas_root: &Path, proj_id: i64, segment: &str) -> PathBuf {
    nas_root.join(format!("proj_{proj_id}/sessions/{segment}"))
}

/// Gateway NAS read path: `proj_{N}/sessions/{segment}/interactive-session.jsonl`.
#[must_use]
pub fn ovs_interactive_session_jsonl_host(nas_root: &Path, proj_id: i64, segment: &str) -> PathBuf {
    ovs_interactive_session_dir_host(nas_root, proj_id, segment)
        .join(OVS_INTERACTIVE_SESSION_FILENAME)
}

/// Guest symlink path: `proj_{N}/home/.claw/interactive/{segment}` → `../../../sessions/{segment}`.
#[must_use]
pub fn ovs_interactive_guest_symlink_host(nas_root: &Path, proj_id: i64, segment: &str) -> PathBuf {
    nas_root.join(format!(
        "proj_{proj_id}/home/{OVS_INTERACTIVE_GUEST_REL_PREFIX}/{segment}"
    ))
}

/// Symlink target relative to `home/.claw/interactive/{segment}`.
#[must_use]
pub fn ovs_interactive_symlink_target(segment: &str) -> String {
    format!("../../../sessions/{segment}")
}

/// Guest absolute path for interactive resume jsonl (via symlink into session dir).
#[must_use]
pub fn ovs_interactive_session_jsonl_guest(segment: &str) -> String {
    format!(
        "{OVS_INTERACTIVE_PROJ_HOME}/{OVS_INTERACTIVE_GUEST_REL_PREFIX}/{segment}/{OVS_INTERACTIVE_SESSION_FILENAME}"
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

/// Idempotent: pick guest jsonl path, mkdir when needed, bootstrap `session_meta` (+ legacy migration).
#[must_use]
pub fn build_ensure_ovs_interactive_session_script(segment: &str) -> String {
    let seg = shell_single_quote(segment.trim());
    format!(
        r#"set -euo pipefail
SEG={seg}
INTERACTIVE_JSONL="{OVS_INTERACTIVE_PROJ_HOME}/{OVS_INTERACTIVE_GUEST_REL_PREFIX}/${{SEG}}/{OVS_INTERACTIVE_SESSION_FILENAME}"
SESSIONS_JSONL="{OVS_INTERACTIVE_GUEST_SESSIONS_MOUNT}/${{SEG}}/{OVS_INTERACTIVE_SESSION_FILENAME}"
LEGACY_JSONL="{OVS_INTERACTIVE_PROJ_HOME}/{OVS_INTERACTIVE_LEGACY_GUEST_REL_PREFIX}/${{SEG}}/{OVS_INTERACTIVE_SESSION_FILENAME}"
if [ -d "{OVS_INTERACTIVE_GUEST_SESSIONS_MOUNT}" ]; then
  JSONL="$SESSIONS_JSONL"
elif [ -d "{OVS_INTERACTIVE_PROJ_HOME}/sessions" ]; then
  JSONL="{OVS_INTERACTIVE_PROJ_HOME}/sessions/${{SEG}}/{OVS_INTERACTIVE_SESSION_FILENAME}"
else
  JSONL="$INTERACTIVE_JSONL"
fi
JSONL_DIR="$(dirname "$JSONL")"
if [ ! -e "$JSONL_DIR" ] && [ ! -L "$JSONL_DIR" ]; then
  mkdir -p "$JSONL_DIR"
fi
if [ -f "$LEGACY_JSONL" ] && [ ! -f "$JSONL" ]; then
  cp -f "$LEGACY_JSONL" "$JSONL"
fi
if [ ! -f "$JSONL" ]; then
  NOW_MS=$(($(date +%s) * 1000))
  SID="ovs-interactive-${{SEG}}"
  printf '%s\n' "{{\"type\":\"session_meta\",\"session_id\":\"${{SID}}\",\"version\":1,\"created_at_ms\":${{NOW_MS}},\"updated_at_ms\":${{NOW_MS}},\"workspace_root\":\"{OVS_INTERACTIVE_PROJ_HOME}\"}}" > "$JSONL"
fi"#
    )
}

/// One-shot OVS agent turn: ensure jsonl + `claw --resume` with web CDP on stdout.
#[must_use]
pub fn build_ovs_interactive_prompt_script(segment: &str, prompt: &str) -> String {
    let seg = shell_single_quote(segment.trim());
    let prompt_b64 = base64::engine::general_purpose::STANDARD.encode(prompt.as_bytes());
    let ensure = build_ensure_ovs_interactive_session_script(segment);
    format!(
        r#"{ensure}
export HOME={OVS_INTERACTIVE_WORK_ROOT}
export CLAW_PROJECT_CONFIG_ROOT={OVS_INTERACTIVE_PROJ_HOME}
export CLAW_GATEWAY_WORK_ROOT={OVS_INTERACTIVE_WORK_ROOT}
export CLAW_DISPLAY_MODE=web
export XDG_CONFIG_HOME={OVS_INTERACTIVE_WORK_ROOT}/.config
export XDG_CACHE_HOME={OVS_INTERACTIVE_WORK_ROOT}/.cache
export XDG_DATA_HOME={OVS_INTERACTIVE_WORK_ROOT}/.local/share
mkdir -p {OVS_INTERACTIVE_WORK_ROOT}/.config {OVS_INTERACTIVE_WORK_ROOT}/.cache {OVS_INTERACTIVE_WORK_ROOT}/.local/share
if [ -f {OVS_INTERACTIVE_WORK_ROOT}/.claw/terminal-llm.env ]; then
  set -a
  # shellcheck source=/dev/null
  . {OVS_INTERACTIVE_WORK_ROOT}/.claw/terminal-llm.env
  set +a
fi
SEG={seg}
if [ -d "{OVS_INTERACTIVE_GUEST_SESSIONS_MOUNT}" ]; then
  JSONL="{OVS_INTERACTIVE_GUEST_SESSIONS_MOUNT}/${{SEG}}/{OVS_INTERACTIVE_SESSION_FILENAME}"
elif [ -d "{OVS_INTERACTIVE_PROJ_HOME}/sessions" ]; then
  JSONL="{OVS_INTERACTIVE_PROJ_HOME}/sessions/${{SEG}}/{OVS_INTERACTIVE_SESSION_FILENAME}"
else
  JSONL="{OVS_INTERACTIVE_PROJ_HOME}/{OVS_INTERACTIVE_GUEST_REL_PREFIX}/${{SEG}}/{OVS_INTERACTIVE_SESSION_FILENAME}"
fi
PROMPT=$(printf '%s' '{prompt_b64}' | base64 -d)
cd {OVS_INTERACTIVE_PROJ_HOME}
MODEL="${{CLAW_DEFAULT_MODEL:-openai/mimo-v2.5}}"
exec claw --allow-broad-cwd --model "$MODEL" --resume "$JSONL" -p "$PROMPT""#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_jsonl_path_uses_interactive_symlink_prefix() {
        let p = ovs_interactive_session_jsonl_guest("ovs-chat-3-abc");
        assert_eq!(
            p,
            "/claw_ds/.claw/interactive/ovs-chat-3-abc/interactive-session.jsonl"
        );
    }

    #[test]
    fn host_jsonl_path_under_sessions_segment_dir() {
        let root = Path::new("/nas");
        let p = ovs_interactive_session_jsonl_host(root, 3, "seg-1");
        assert_eq!(
            p,
            Path::new("/nas/proj_3/sessions/seg-1/interactive-session.jsonl")
        );
        assert_eq!(
            ovs_interactive_session_dir_host(root, 3, "seg-1"),
            Path::new("/nas/proj_3/sessions/seg-1")
        );
    }

    #[test]
    fn guest_symlink_target_points_at_sessions_tree() {
        assert_eq!(
            ovs_interactive_symlink_target("ovs-chat-3-x"),
            "../../../sessions/ovs-chat-3-x"
        );
        let root = Path::new("/nas");
        assert_eq!(
            ovs_interactive_guest_symlink_host(root, 2, "seg"),
            Path::new("/nas/proj_2/home/.claw/interactive/seg")
        );
    }

    #[test]
    fn ensure_script_bootstraps_meta_and_migrates_legacy() {
        let sh = build_ensure_ovs_interactive_session_script("ovs-chat-1-x");
        assert!(sh.contains("ovs-chat-1-x"));
        assert!(sh.contains("interactive-session.jsonl"));
        assert!(sh.contains(".claw/interactive"));
        assert!(sh.contains(".claw/ovs-chat"));
        assert!(sh.contains("LEGACY_JSONL"));
        assert!(sh.contains("session_meta"));
        assert!(sh.contains("/claw_ds"));
        assert!(sh.contains("/claw_sessions"));
        assert!(
            sh.contains(r#"if [ ! -e "$JSONL_DIR" ] && [ ! -L "$JSONL_DIR" ]; then"#),
            "must skip mkdir when gateway symlink already exists"
        );
    }

    #[test]
    fn prompt_script_uses_base64_and_resume() {
        let sh = build_ovs_interactive_prompt_script("seg-a", "hello \"world\"\nline2");
        assert!(sh.contains("base64 -d"));
        assert!(sh.contains("--resume"));
        assert!(sh.contains("CLAW_DISPLAY_MODE=web"));
        assert!(sh.contains(".claw/interactive"));
        assert!(sh.contains("seg-a"));
        assert!(!sh.contains("hello \"world\""));
        assert!(
            !sh.contains(".claw/sessions"),
            "must not use default SessionStore path"
        );
    }

    #[test]
    fn prompt_script_escapes_single_quotes_in_segment() {
        let sh = build_ovs_interactive_prompt_script("it's", "ok");
        assert!(sh.contains("'it'\"'\"'s'"));
    }
}
