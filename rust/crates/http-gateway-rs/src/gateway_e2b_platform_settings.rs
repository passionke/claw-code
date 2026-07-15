//! Admin read-only e2b platform view from process env (repo `.env` → gateway restart). Author: kejiqing

use serde::Serialize;

use crate::gateway_e2b_worker_settings::{
    e2b_worker_relaxed_template_from_env, e2b_worker_template_from_env,
};
use crate::pool::relaxed_worker_allowed_from_env;

#[derive(Debug, Clone, Serialize)]
pub struct E2bPlatformSettingsPublic {
    #[serde(rename = "readOnly")]
    pub read_only: bool,
    #[serde(rename = "e2bApiUrl")]
    pub e2b_api_url: String,
    #[serde(rename = "e2bSandboxUrl", skip_serializing_if = "Option::is_none")]
    pub e2b_sandbox_url: Option<String>,
    #[serde(rename = "e2bDomain")]
    pub e2b_domain: String,
    #[serde(rename = "apiKeySet")]
    pub api_key_set: bool,
    #[serde(rename = "workerStrictTemplate")]
    pub worker_strict_template: String,
    #[serde(rename = "workerRelaxedTemplate")]
    pub worker_relaxed_template: String,
    #[serde(rename = "sandboxTimeoutSecs")]
    pub sandbox_timeout_secs: u64,
    /// `CLAW_ALLOW_RELAXED_WORKER` — false hides Admin relaxed option and rejects API writes.
    #[serde(rename = "relaxedWorkerAllowed")]
    pub relaxed_worker_allowed: bool,
    pub configured: bool,
}

fn env_trim(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn e2b_api_url_from_env() -> String {
    env_trim("CLAW_E2B_API_URL")
        .unwrap_or_else(|| "https://api.cn-beijing.e2b.fc.aliyuncs.com".into())
}

fn e2b_domain_from_env() -> String {
    env_trim("CLAW_E2B_DOMAIN")
        .or_else(|| env_trim("E2B_DOMAIN"))
        .unwrap_or_else(|| "cn-beijing.e2b.fc.aliyuncs.com".into())
}

fn e2b_sandbox_url_from_env() -> Option<String> {
    env_trim("CLAW_E2B_SANDBOX_URL").or_else(|| env_trim("E2B_SANDBOX_URL"))
}

fn e2b_api_key_set() -> bool {
    env_trim("CLAW_E2B_API_KEY")
        .or_else(|| env_trim("ALIYUN_E2B_TOKEN"))
        .is_some()
}

fn sandbox_timeout_secs_from_env() -> u64 {
    env_trim("CLAW_E2B_SANDBOX_TIMEOUT_SECS")
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(3600)
}

/// Snapshot for Admin `GET /v1/gateway/global-settings` → `e2bPlatform` (env-only, not PG).
#[must_use]
pub fn e2b_platform_settings_public() -> E2bPlatformSettingsPublic {
    let e2b_api_url = e2b_api_url_from_env();
    let api_key_set = e2b_api_key_set();
    E2bPlatformSettingsPublic {
        read_only: true,
        e2b_sandbox_url: e2b_sandbox_url_from_env(),
        e2b_domain: e2b_domain_from_env(),
        worker_strict_template: e2b_worker_template_from_env(),
        worker_relaxed_template: e2b_worker_relaxed_template_from_env(),
        sandbox_timeout_secs: sandbox_timeout_secs_from_env(),
        relaxed_worker_allowed: relaxed_worker_allowed_from_env(),
        configured: api_key_set && !e2b_api_url.is_empty(),
        api_key_set,
        e2b_api_url,
    }
}
