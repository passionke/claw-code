//! Pool worker: declare consumed env keys and load from mounted repo `.env` at solve start. Author: kejiqing

use std::collections::HashSet;
use std::path::PathBuf;

use api::apply_dotenv_keys_from_paths;

/// In-container path when the pool bind-mounts host `CLAW_WORKER_ENV_FILE`. Author: kejiqing
pub const WORKER_ENV_MOUNT_PATH: &str = "/run/claw/worker.env";

/// Keys the solve worker reads during `gateway-solve-once` (provider, MCP, prompts, progress).
/// Add new worker-facing vars here — not in deploy shell ALLOW lists. Author: kejiqing
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
    "CLAW_MCP_PARALLEL_FANOUT",
    "CLAW_INSTRUCTION_FILE_MAX_CHARS",
    "CLAW_INSTRUCTION_TOTAL_MAX_CHARS",
    "CLAW_PROGRESS_MESSAGE_MAX_CHARS",
    "CLAW_GATEWAY_SQLBOT_PREFLIGHT",
    "CLAW_GATEWAY_INTERNAL_BASE_URL",
    "CLAW_GATEWAY_INTERNAL_TOKEN",
    "CLAW_WORKER_REPORT_SSE_PORT",
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

#[must_use]
pub fn worker_env_keys_set() -> HashSet<&'static str> {
    WORKER_ENV_KEYS.iter().copied().collect()
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
}
