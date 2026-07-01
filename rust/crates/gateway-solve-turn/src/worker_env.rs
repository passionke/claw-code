//! Pool worker: declare consumed env keys and load from mounted repo `.env` at solve start. Author: kejiqing

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use api::apply_dotenv_keys_from_paths;
use base64::Engine;

/// Guest path relative to `CLAW_GATEWAY_WORK_ROOT` / `/claw_host_root`.
/// Gateway writes **dialogue** `record_session_id` here before each OVS `@claw` prompt;
/// `claw` REPL reads it on every LLM call (warm workers cannot rely on process env).
/// Same id is sent as `claw-session-id` to co-located tap — **no new session model**.
/// See `docs/ovs-chat/OVS-INTERACTIVE-SESSION-ID.md`. Author: kejiqing
pub const GATEWAY_RECORD_SESSION_ID_REL: &str = ".claw/gateway-record-session-id";

/// In-container absolute path (FC interactive + podman pool workers). Author: kejiqing
pub const GATEWAY_RECORD_SESSION_ID_GUEST: &str = "/claw_host_root/.claw/gateway-record-session-id";

/// In-container path when the pool bind-mounts host `CLAW_WORKER_ENV_FILE`. Author: kejiqing
pub const WORKER_ENV_MOUNT_PATH: &str = "/run/claw/worker.env";

/// Keys the solve worker reads during `gateway-solve-once` (provider, MCP, prompts, progress).
/// Add new worker-facing vars here — not in deploy shell ALLOW lists.
/// Never add `CLAW_GATEWAY_DATABASE_URL` / `POSTGRES_*` — workers do not connect to PostgreSQL. Author: kejiqing
pub const WORKER_ENV_KEYS: &[&str] = &[
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_PROJECT_ID",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "XAI_API_KEY",
    "OPENROUTER_API_KEY",
    "CLAW_DEFAULT_MODEL",
    "ANTHROPIC_MODEL",
    "CLAW_OPENAI_FALLBACK_MODEL",
    "CLAW_PREFER_OPENAI_PREFIX",
    "CLAW_DISABLE_ANTHROPIC_ROUTING",
    "CLAW_MCP_TOOL_CALL_TIMEOUT_MS",
    "CLAW_MCP_MAX_CONCURRENT",
    "CLAW_INSTRUCTION_FILE_MAX_CHARS",
    "CLAW_INSTRUCTION_TOTAL_MAX_CHARS",
    "CLAW_PROGRESS_MESSAGE_MAX_CHARS",
    "CLAW_GATEWAY_INTERNAL_BASE_URL",
    "CLAW_GATEWAY_INTERNAL_TOKEN",
    "CLAW_POOL_ID",
    "CLAW_SESSION_ID",
    "CLAW_TURN_ID",
    "CLAW_WORKER_NAME",
    "CLAW_SSE_BURST_TRACE",
    "CLAW_SSE_BURST_LOG_FILE",
    "CLAW_OTEL_ENABLED",
    "CLAW_OTEL_LOG_PROMPTS",
    "LANGFUSE_PUBLIC_KEY",
    "LANGFUSE_SECRET_KEY",
    "LANGFUSE_BASE_URL",
];

fn worker_env_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(raw) = std::env::var("CLAW_WORKER_ENV_FILE") {
        for part in raw.split(':') {
            let p = part.trim();
            if !p.is_empty() {
                paths.push(PathBuf::from(p));
            }
        }
    }
    paths.push(PathBuf::from(WORKER_ENV_MOUNT_PATH));
    paths
}

/// Load declared [`WORKER_ENV_KEYS`] from `CLAW_WORKER_ENV_FILE` / `/run/claw/worker.env`.
/// Existing process env wins (compose `env_file`, `docker exec -e`, exports). Author: kejiqing
pub fn apply_worker_env() {
    apply_dotenv_keys_from_paths(&worker_env_search_paths(), WORKER_ENV_KEYS);
}

/// Langfuse / OTEL keys to forward into worker `docker exec -e` (pool host may not inherit compose env).
#[must_use]
pub fn otel_forward_env() -> BTreeMap<String, String> {
    const KEYS: &[&str] = &[
        "CLAW_OTEL_ENABLED",
        "CLAW_OTEL_LOG_PROMPTS",
        "LANGFUSE_PUBLIC_KEY",
        "LANGFUSE_SECRET_KEY",
        "LANGFUSE_BASE_URL",
    ];
    let mut out = BTreeMap::new();
    for key in KEYS {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                out.insert((*key).to_string(), value);
            }
        }
    }
    out
}

#[must_use]
pub fn worker_env_keys_set() -> HashSet<&'static str> {
    WORKER_ENV_KEYS.iter().copied().collect()
}

/// Dialogue session id for tap / Admin traces (reuse solve header contract).
/// Priority: `CLAW_SESSION_ID` (solve `docker exec -e`) then gateway record file.
#[must_use]
pub fn resolve_gateway_llm_session_id() -> Option<String> {
    if let Ok(raw) = std::env::var("CLAW_SESSION_ID") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let root = std::env::var("CLAW_GATEWAY_WORK_ROOT")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())?;
    read_gateway_record_session_id_file(Path::new(&root))
}

fn read_gateway_record_session_id_file(work_root: &Path) -> Option<String> {
    let path = work_root.join(GATEWAY_RECORD_SESSION_ID_REL);
    let raw = std::fs::read_to_string(&path).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// LLM outbound headers — same keys as [`crate::DirectApiClient`] / `/v1/solve`. Author: kejiqing
#[must_use]
pub fn gateway_llm_session_extra_headers() -> BTreeMap<String, String> {
    let Some(session_id) = resolve_gateway_llm_session_id() else {
        return BTreeMap::new();
    };
    BTreeMap::from([
        ("clawcode-session-id".to_string(), session_id.clone()),
        ("claw-session-id".to_string(), session_id),
    ])
}

/// Idempotent guest shell: stage dialogue `record_session_id` for the next `claw` LLM call. Author: kejiqing
#[must_use]
pub fn build_write_gateway_record_session_script(record_session_id: &str) -> String {
    let trimmed = record_session_id.trim();
    let b64 = base64::engine::general_purpose::STANDARD.encode(trimmed.as_bytes());
    format!(
        r"set -e
mkdir -p /claw_host_root/.claw
printf '%s' '{b64}' | base64 -d > {GATEWAY_RECORD_SESSION_ID_GUEST}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn apply_worker_env_reads_only_declared_keys() {
        let _guard = env_lock();
        let dir = std::env::temp_dir().join(format!("claw-worker-env-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let env_path = dir.join("worker.env");
        std::fs::write(
            &env_path,
            "OPENAI_API_KEY=sk-test\nGATEWAY_IMAGE=must-not-leak\nCLAW_PROGRESS_MESSAGE_MAX_CHARS=99\n",
        )
        .unwrap();

        let key = "OPENAI_API_KEY";
        let prev: Option<OsString> = std::env::var_os(key);
        std::env::remove_var(key);
        std::env::remove_var("CLAW_PROGRESS_MESSAGE_MAX_CHARS");
        std::env::remove_var("GATEWAY_IMAGE");

        std::env::set_var("CLAW_WORKER_ENV_FILE", env_path.display().to_string());
        apply_worker_env();
        assert_eq!(std::env::var(key).unwrap(), "sk-test");
        assert_eq!(
            std::env::var("CLAW_PROGRESS_MESSAGE_MAX_CHARS").unwrap(),
            "99"
        );
        assert!(std::env::var("GATEWAY_IMAGE").is_err());

        match prev {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
        std::env::remove_var("CLAW_WORKER_ENV_FILE");
        std::env::remove_var("CLAW_PROGRESS_MESSAGE_MAX_CHARS");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_gateway_llm_session_id_prefers_claw_session_id_env() {
        let _guard = env_lock();
        let prev = std::env::var("CLAW_SESSION_ID").ok();
        std::env::set_var("CLAW_SESSION_ID", "ovs-chat-1-abc");
        assert_eq!(
            resolve_gateway_llm_session_id().as_deref(),
            Some("ovs-chat-1-abc")
        );
        match prev {
            Some(v) => std::env::set_var("CLAW_SESSION_ID", v),
            None => std::env::remove_var("CLAW_SESSION_ID"),
        }
    }

    #[test]
    fn resolve_gateway_llm_session_id_reads_record_file() {
        let _guard = env_lock();
        std::env::remove_var("CLAW_SESSION_ID");
        let dir = std::env::temp_dir().join(format!("claw-record-sid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".claw")).unwrap();
        std::fs::write(
            dir.join(GATEWAY_RECORD_SESSION_ID_REL),
            "ovs-chat-2-deadbeef\n",
        )
        .unwrap();
        let prev_root = std::env::var("CLAW_GATEWAY_WORK_ROOT").ok();
        std::env::set_var("CLAW_GATEWAY_WORK_ROOT", dir.display().to_string());
        assert_eq!(
            resolve_gateway_llm_session_id().as_deref(),
            Some("ovs-chat-2-deadbeef")
        );
        match prev_root {
            Some(v) => std::env::set_var("CLAW_GATEWAY_WORK_ROOT", v),
            None => std::env::remove_var("CLAW_GATEWAY_WORK_ROOT"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gateway_llm_session_extra_headers_match_solve_contract() {
        let _guard = env_lock();
        std::env::set_var("CLAW_SESSION_ID", "sess-x");
        let h = gateway_llm_session_extra_headers();
        assert_eq!(h.get("claw-session-id").map(String::as_str), Some("sess-x"));
        assert_eq!(
            h.get("clawcode-session-id").map(String::as_str),
            Some("sess-x")
        );
        std::env::remove_var("CLAW_SESSION_ID");
    }

    #[test]
    fn write_gateway_record_session_script_targets_guest_path() {
        let sh = build_write_gateway_record_session_script("ovs-chat-3-afc29");
        assert!(sh.contains(GATEWAY_RECORD_SESSION_ID_GUEST));
        assert!(sh.contains("base64 -d"));
    }
}
