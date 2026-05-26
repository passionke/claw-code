//! Apply gateway LLM model settings to repo `.env` and refresh claude-tap chain. Author: kejiqing

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Serialize;
use tokio::process::Command;

/// Key in `git_pat_tokens_json` for the global LLM API key (not a Git PAT). Author: kejiqing
pub const LLM_API_KEY_STORE_ID: &str = "__gateway_llm_api_key__";

#[derive(Debug, Clone, Serialize)]
pub struct LlmModelApplyOutcome {
    #[serde(rename = "envFile")]
    pub env_file: String,
    #[serde(rename = "appliedAtMs")]
    pub applied_at_ms: i64,
    #[serde(rename = "tapChainRefreshed")]
    pub tap_chain_refreshed: bool,
    #[serde(rename = "tapRestarted")]
    pub tap_restarted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Resolve repo-root `.env` from `CLAW_WORKER_ENV_FILE` or `CLAW_REPO_ROOT`. Author: kejiqing
#[must_use]
pub fn resolve_repo_env_file() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CLAW_WORKER_ENV_FILE") {
        let p = PathBuf::from(raw.trim());
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(root) = std::env::var("CLAW_REPO_ROOT") {
        let p = PathBuf::from(root.trim()).join(".env");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn resolve_repo_root(env_file: &Path) -> PathBuf {
    env_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// OpenAI-compat upstream base for claude-tap + claw (`OPENAI_BASE_URL` + `/chat/completions`).
/// Keep a trailing `/v1` when the provider expects `…/v1/chat/completions` (e.g. xiaomimimo).
/// Author: kejiqing
#[must_use]
pub fn normalize_upstream_base_url(raw: &str) -> Option<String> {
    let s = raw.trim().trim_end_matches('/').to_string();
    if s.is_empty() {
        return None;
    }
    if !s.starts_with("http://") && !s.starts_with("https://") {
        return None;
    }
    Some(s)
}

#[must_use]
pub fn normalize_model_name(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 256 {
        return None;
    }
    Some(s.to_string())
}

/// Map Admin typos / product display names → upstream `model` id. Author: kejiqing
#[must_use]
pub fn normalize_model_name_for_upstream(raw: &str, upstream_base_url: &str) -> Option<String> {
    let model = normalize_model_name(raw)?;
    let host = upstream_base_url.to_ascii_lowercase();
    if !host.contains("xiaomimimo") {
        return Some(model);
    }
    let bare = model.strip_prefix("openai/").unwrap_or(model.as_str());
    let key = bare.to_ascii_lowercase().replace('_', "-");
    let mapped = match key.as_str() {
        "mimo-v2.5-pro" | "mimo-v2.5" => "mimo-v2.5-pro",
        "mimo-v2.5-flash" => "mimo-v2.5-flash",
        "mimo-v2-pro" => "mimo-v2-pro",
        _ => bare,
    };
    Some(mapped.to_string())
}

fn fmt_env_line(key: &str, value: &str) -> String {
    let needs_quote = value
        .chars()
        .any(|c| c.is_whitespace() || c == '#' || c == '\'')
        || value.starts_with('-');
    if needs_quote {
        let escaped = value.replace('\'', "'\"'\"'");
        format!("{key}='{escaped}'\n")
    } else {
        format!("{key}={value}\n")
    }
}

/// Upsert one `KEY=value` line in a dotenv file (preserves unrelated lines). Author: kejiqing
pub fn upsert_dotenv_kv(path: &Path, key: &str, value: &str) -> Result<(), String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut lines: Vec<String> = content.lines().map(|l| format!("{l}\n")).collect();
    if !content.is_empty() && !content.ends_with('\n') {
        if let Some(last) = lines.last_mut() {
            if !last.ends_with('\n') {
                last.push('\n');
            }
        }
    }
    let prefix = format!("{key}=");
    let new_line = fmt_env_line(key, value);
    let mut seen = false;
    for line in &mut lines {
        if line.starts_with(&prefix) || line.starts_with(&format!("export {prefix}")) {
            if !seen {
                *line = new_line.clone();
                seen = true;
            } else {
                *line = String::new();
            }
        }
    }
    lines.retain(|l| !l.is_empty());
    if !seen {
        lines.push(new_line);
    }
    std::fs::write(path, lines.concat()).map_err(|e| format!("write {}: {e}", path.display()))
}

pub async fn apply_llm_model_to_env(
    env_file: &Path,
    upstream_base_url: &str,
    model_name: &str,
    api_key: &str,
) -> Result<LlmModelApplyOutcome, String> {
    let upstream = normalize_upstream_base_url(upstream_base_url)
        .ok_or_else(|| "invalid baseModelUrl (expect http(s):// host; use …/v1 when provider needs /v1/chat/completions)".to_string())?;
    let model = normalize_model_name_for_upstream(model_name, upstream_base_url)
        .ok_or_else(|| "modelName is required".to_string())?;
    if api_key.trim().is_empty() {
        return Err("apiKey is not configured".into());
    }

    upsert_dotenv_kv(env_file, "UPSTREAM_OPENAI_BASE_URL", &upstream)?;
    upsert_dotenv_kv(env_file, "OPENAI_API_KEY", api_key.trim())?;
    upsert_dotenv_kv(env_file, "CLAW_DEFAULT_MODEL", &model)?;
    upsert_dotenv_kv(env_file, "ANTHROPIC_MODEL", &model)?;
    upsert_dotenv_kv(env_file, "CLAW_OPENAI_FALLBACK_MODEL", &model)?;

    let repo_root = resolve_repo_root(env_file);
    let refresh_py = repo_root.join("deploy/stack/refresh-tap-llm-chain-in-env.py");
    let mut tap_chain_refreshed = false;
    if refresh_py.is_file() {
        let status = Command::new("python3")
            .arg(&refresh_py)
            .arg(env_file)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .status()
            .await
            .map_err(|e| format!("run refresh-tap-llm-chain: {e}"))?;
        if !status.success() {
            return Err(format!(
                "refresh-tap-llm-chain-in-env.py exited with {}",
                status.code().unwrap_or(-1)
            ));
        }
        tap_chain_refreshed = true;
    }

    let stack_dir = repo_root.join("deploy/stack");
    let compose_include = stack_dir.join("lib/compose-include.sh");
    if compose_include.is_file() {
        let ensure = format!(
            "set -euo pipefail; source '{}'; set -a; source '{}/.env'; set +a; claw_ensure_worker_llm_wiring '{}'",
            compose_include.display(),
            repo_root.display(),
            stack_dir.display()
        );
        let status = Command::new("bash")
            .arg("-c")
            .arg(&ensure)
            .current_dir(&repo_root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .status()
            .await
            .map_err(|e| format!("run claw_ensure_worker_llm_wiring: {e}"))?;
        if !status.success() {
            return Err(format!(
                "claw_ensure_worker_llm_wiring exited with {}",
                status.code().unwrap_or(-1)
            ));
        }
    }

    // Upstream is hot-reloaded via claude-tap `--tap-upstream-config` file; restart only when requested.
    let enable_restart = matches!(
        std::env::var("CLAW_GATEWAY_LLM_APPLY_RESTART_TAP")
            .ok()
            .map(|v| v.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "on" | "yes")
    );
    let mut tap_restarted = false;
    let mut message = None;
    if enable_restart {
        let sync_sh = repo_root.join("deploy/stack/lib/sync-worker-openai-env.sh");
        if sync_sh.is_file() {
            let output = Command::new("bash")
                .arg(&sync_sh)
                .arg("--restart")
                .current_dir(&repo_root)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| format!("run sync-worker-openai-env.sh: {e}"))?;
            tap_restarted = output.status.success();
            if !tap_restarted {
                message = Some(format!(
                    "gateway stack restart after LLM apply failed (code {:?}); .env was updated",
                    output.status.code()
                ));
            }
        } else {
            message = Some(
                "deploy/stack/lib/sync-worker-openai-env.sh not found; .env updated only".into(),
            );
        }
    } else {
        message = Some(
            "tap restart skipped (upstream file hot-reload; set CLAW_GATEWAY_LLM_APPLY_RESTART_TAP=1 to force restart)"
                .into(),
        );
    }

    let applied_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));

    Ok(LlmModelApplyOutcome {
        env_file: env_file.display().to_string(),
        applied_at_ms,
        tap_chain_refreshed,
        tap_restarted,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn normalize_upstream_preserves_v1_suffix() {
        assert_eq!(
            normalize_upstream_base_url("https://token-plan-cn.xiaomimimo.com/v1/").as_deref(),
            Some("https://token-plan-cn.xiaomimimo.com/v1")
        );
        assert_eq!(
            normalize_upstream_base_url("https://api.deepseek.com").as_deref(),
            Some("https://api.deepseek.com")
        );
    }

    #[test]
    fn xiaomi_model_display_name_maps_to_api_id() {
        let base = "https://token-plan-cn.xiaomimimo.com/v1";
        assert_eq!(
            normalize_model_name_for_upstream("MiMo-V2.5-Pro", base).as_deref(),
            Some("mimo-v2.5-pro")
        );
        assert_eq!(
            normalize_model_name_for_upstream("openai/mimo-v2.5-pro", base).as_deref(),
            Some("mimo-v2.5-pro")
        );
        assert_eq!(
            normalize_model_name_for_upstream("deepseek-chat", "https://api.deepseek.com")
                .as_deref(),
            Some("deepseek-chat")
        );
    }

    #[test]
    fn upsert_dotenv_replaces_existing_key() {
        let dir = std::env::temp_dir().join(format!("claw-llm-env-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        fs::write(&path, "FOO=1\nUPSTREAM_OPENAI_BASE_URL=http://old\n").unwrap();
        upsert_dotenv_kv(&path, "UPSTREAM_OPENAI_BASE_URL", "https://api.example.com").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("UPSTREAM_OPENAI_BASE_URL=https://api.example.com"));
        assert!(!text.contains("http://old"));
        let _ = fs::remove_dir_all(&dir);
    }
}
