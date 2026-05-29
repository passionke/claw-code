//! Poll PostgreSQL active LLM model → in-memory runtime + claude-tap upstream file + worker `.env`.
//! Author: kejiqing

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::gateway_global_settings::{self, ActiveLlmRuntime};
use crate::gateway_llm_model_apply::{
    apply_llm_model_to_env, normalize_model_name_for_upstream, normalize_upstream_base_url,
    resolve_llm_runtime_env_file, resolve_repo_env_file, LlmModelApplyOutcome,
};
use crate::session_db::GatewaySessionDb;

/// In-memory active LLM config (DB is source of truth; refreshed on apply + poll). Author: kejiqing
#[derive(Debug, Clone)]
pub struct LlmRuntimeConfig {
    pub model_id: String,
    pub model_rev: String,
    pub upstream_base_url: String,
    pub model_name: String,
    pub api_key: String,
    pub applied_at_ms: Option<i64>,
}

pub type LlmRuntimeHandle = Arc<RwLock<Option<LlmRuntimeConfig>>>;

#[derive(Debug, Clone, Serialize)]
pub struct LlmConfigSyncOutcome {
    pub changed: bool,
    #[serde(rename = "upstreamConfigFile", skip_serializing_if = "Option::is_none")]
    pub upstream_config_file: Option<String>,
    #[serde(rename = "upstreamFileWritten")]
    pub upstream_file_written: bool,
    #[serde(rename = "envApply", skip_serializing_if = "Option::is_none")]
    pub env_apply: Option<LlmModelApplyOutcome>,
}

/// Path for claude-tap `--tap-upstream-config` (hot reload). Author: kejiqing
#[must_use]
pub fn resolve_claude_tap_upstream_config_path() -> PathBuf {
    if let Ok(raw) = std::env::var("CLAW_TAP_UPSTREAM_CONFIG_FILE") {
        let p = PathBuf::from(raw.trim());
        if !p.as_os_str().is_empty() {
            return p;
        }
    }
    if let Ok(root) = std::env::var("CLAW_REPO_ROOT") {
        return PathBuf::from(root.trim()).join(".claw/claw-tap-upstream.json");
    }
    if let Some(env_file) = resolve_repo_env_file() {
        if let Some(parent) = env_file.parent() {
            return parent.join(".claw/claw-tap-upstream.json");
        }
    }
    PathBuf::from(".claw/claw-tap-upstream.json")
}

fn upstream_config_desired_text(target: &str) -> String {
    let v = serde_json::json!({ "target": target });
    serde_json::to_string_pretty(&v).unwrap_or_else(|_| format!(r#"{{"target":"{target}"}}"#))
}

/// Write `{"target":"https://..."}` for claude-tap hot reload. Skips write when unchanged. Author: kejiqing
pub fn write_claude_tap_upstream_config(path: &Path, target: &str) -> Result<bool, String> {
    let upstream = normalize_upstream_base_url(target)
        .ok_or_else(|| "invalid upstream base URL for claude-tap config file".to_string())?;
    let desired = upstream_config_desired_text(&upstream);
    if path.is_file() {
        if let Ok(cur) = std::fs::read_to_string(path) {
            if cur.trim() == desired.trim() {
                return Ok(false);
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    std::fs::write(path, format!("{desired}\n"))
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(true)
}

fn runtime_from_active(active: &ActiveLlmRuntime) -> Option<LlmRuntimeConfig> {
    let upstream = normalize_upstream_base_url(&active.base_model_url)?;
    let model_name = normalize_model_name_for_upstream(&active.model_name, &active.base_model_url)?;
    let api_key = active.api_key.trim();
    if api_key.is_empty() {
        return None;
    }
    Some(LlmRuntimeConfig {
        model_id: active.model_id.clone(),
        model_rev: active.model_rev.clone(),
        upstream_base_url: upstream,
        model_name,
        api_key: api_key.to_string(),
        applied_at_ms: active.applied_at_ms,
    })
}

fn same_runtime(a: &LlmRuntimeConfig, b: &LlmRuntimeConfig) -> bool {
    a.model_id == b.model_id
        && a.model_rev == b.model_rev
        && a.upstream_base_url == b.upstream_base_url
        && a.model_name == b.model_name
        && a.api_key == b.api_key
}

/// Read active LLM from DB; update memory, upstream JSON, and worker `.env` when changed. Author: kejiqing
pub async fn sync_llm_runtime_from_db(
    db: &GatewaySessionDb,
    handle: &LlmRuntimeHandle,
) -> Result<LlmConfigSyncOutcome, String> {
    let active = gateway_global_settings::load_active_llm_runtime(db)
        .await
        .map_err(|e| e.to_string())?;
    let Some(active) = active else {
        let mut guard = handle.write().await;
        let had = guard.take().is_some();
        return Ok(LlmConfigSyncOutcome {
            changed: had,
            upstream_config_file: None,
            upstream_file_written: false,
            env_apply: None,
        });
    };

    let Some(next) = runtime_from_active(&active) else {
        return Err("active LLM revision is incomplete (base URL, model name, or api key)".into());
    };

    let upstream_path = resolve_claude_tap_upstream_config_path();
    let upstream_written =
        write_claude_tap_upstream_config(&upstream_path, &next.upstream_base_url)?;

    let mut env_apply = None;
    let env_changed = {
        let guard = handle.read().await;
        guard
            .as_ref()
            .map(|cur| !same_runtime(cur, &next))
            .unwrap_or(true)
    };

    if env_changed {
        let env_file = resolve_llm_runtime_env_file();
        env_apply = Some(
            apply_llm_model_to_env(
                &env_file,
                &next.upstream_base_url,
                &next.model_name,
                &next.api_key,
            )
            .await?,
        );
    }

    let changed = env_changed || upstream_written;
    {
        let mut guard = handle.write().await;
        *guard = Some(next);
    }

    Ok(LlmConfigSyncOutcome {
        changed,
        upstream_config_file: Some(upstream_path.display().to_string()),
        upstream_file_written: upstream_written,
        env_apply,
    })
}

pub async fn run_startup_llm_config_sync(db: &GatewaySessionDb, handle: &LlmRuntimeHandle) {
    match sync_llm_runtime_from_db(db, handle).await {
        Ok(o) if o.changed => info!(
            target: "claw_gateway_orchestration",
            component = "llm_config_sync",
            phase = "startup",
            upstream_file = o.upstream_config_file.as_deref().unwrap_or(""),
            upstream_written = o.upstream_file_written,
            "LLM runtime synced from DB"
        ),
        Ok(_) => {}
        Err(e) => warn!(
            target: "claw_gateway_orchestration",
            component = "llm_config_sync",
            phase = "startup",
            error = %e,
            "LLM runtime startup sync failed"
        ),
    }
}

pub async fn llm_config_poll_loop(
    db: Arc<GatewaySessionDb>,
    handle: LlmRuntimeHandle,
    interval_secs: u64,
) {
    let start = tokio::time::Instant::now() + std::time::Duration::from_secs(interval_secs);
    let mut ticker = tokio::time::interval_at(start, std::time::Duration::from_secs(interval_secs));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        ticker.tick().await;
        match sync_llm_runtime_from_db(&db, &handle).await {
            Ok(o) if o.changed => info!(
                target: "claw_gateway_orchestration",
                component = "llm_config_sync",
                upstream_written = o.upstream_file_written,
                "LLM runtime synced from DB (poll)"
            ),
            Ok(_) => {}
            Err(e) => warn!(
                target: "claw_gateway_orchestration",
                component = "llm_config_sync",
                error = %e,
                "LLM config poll sync failed"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_config_json_shape() {
        let text = upstream_config_desired_text("https://api.deepseek.com");
        assert!(text.contains("\"target\""));
        assert!(text.contains("api.deepseek.com"));
    }

    #[test]
    fn write_upstream_skips_unchanged() {
        let dir = std::env::temp_dir().join(format!("claw-upstream-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".claw/claw-tap-upstream.json");
        assert!(write_claude_tap_upstream_config(&path, "https://api.example.com").unwrap());
        assert!(!write_claude_tap_upstream_config(&path, "https://api.example.com").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
