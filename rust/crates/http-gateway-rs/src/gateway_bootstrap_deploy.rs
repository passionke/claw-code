//! Bootstrap deploy env write (wizard step 1). Template publish: Admin → publish-templates API. Author: kejiqing

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use utoipa::ToSchema;

use crate::gateway_e2b_platform_settings::{self, E2bPlatformSettingsPublic};
use crate::gateway_llm_model_apply::{resolve_repo_env_file, upsert_dotenv_kv};
use crate::session_db::GatewaySessionDb;

const DEPLOY_ENV_WHITELIST: &[&str] = &[
    "CLAW_CLUSTER_ID",
    "CLAW_GATEWAY_DATABASE_URL",
    "CLAW_DEPLOY_PROFILE",
    "CLAW_E2B_API_URL",
    "CLAW_E2B_SANDBOX_URL",
    "CLAW_E2B_API_KEY",
    "ALIYUN_E2B_TOKEN",
    "CLAW_E2B_DOMAIN",
    "CLAW_E2B_NAS_API",
    "GATEWAY_HOST_PORT",
    "GATEWAY_PLAYGROUND_HOST_PORT",
    "PLAYGROUND_PUBLIC_GATEWAY_BASE",
    "CLAW_PG_HOST",
    "CLAW_PG_PORT",
    "CLAW_PG_USER",
    "CLAW_PG_DATABASE",
    "CLAW_BOOTSTRAP_LLM_API_KEY",
    "CLAW_BOOTSTRAP_LLM_BASE_URL",
    "CLAW_BOOTSTRAP_LLM_MODEL_NAME",
    "CLAW_BOOTSTRAP_LLM_NAME",
];

/// Presence flags for wizard UI (writable/repo/build/pg). Author: kejiqing
#[derive(Debug, Clone, Serialize, ToSchema)]
#[allow(clippy::struct_excessive_bools)]
pub struct BootstrapEnvSnapshot {
    #[serde(rename = "clusterId")]
    pub cluster_id: String,
    #[serde(rename = "gatewayDatabaseUrl")]
    pub gateway_database_url: String,
    #[serde(rename = "deployProfile", skip_serializing_if = "Option::is_none")]
    pub deploy_profile: Option<String>,
    #[serde(rename = "e2bPlatform")]
    pub e2b_platform: E2bPlatformSettingsPublic,
    #[serde(rename = "repoRoot", skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    #[serde(rename = "deployEnvPath", skip_serializing_if = "Option::is_none")]
    pub deploy_env_path: Option<String>,
    #[serde(rename = "deployEnvWritable")]
    pub deploy_env_writable: bool,
    #[serde(rename = "repoRootPresent")]
    pub repo_root_present: bool,
    #[serde(rename = "buildScriptPresent")]
    pub build_script_present: bool,
    #[serde(rename = "pgHostPort", skip_serializing_if = "Option::is_none")]
    pub pg_host_port: Option<String>,
    #[serde(rename = "pgRlsManaged")]
    pub pg_rls_managed: bool,
    pub values: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BootstrapEnvValidation {
    #[serde(rename = "pgOk")]
    pub pg_ok: bool,
    #[serde(rename = "e2bOk")]
    pub e2b_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BootstrapApplyDeployEnvResponse {
    pub applied: Vec<String>,
    #[serde(rename = "restartRequired")]
    pub restart_required: bool,
    #[serde(rename = "envFile")]
    pub env_file: String,
    pub validation: BootstrapEnvValidation,
    /// True when e2b API/domain/sandbox URL changed — PG template pins cleared. Author: kejiqing
    #[serde(rename = "templatesInvalidated", default)]
    pub templates_invalidated: bool,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BootstrapApplyDeployEnvInput {
    #[serde(default)]
    pub values: std::collections::BTreeMap<String, String>,
}

fn env_trim(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Deploy `.env` path: `CLAW_DEPLOY_ENV_FILE` → `/run/claw/deploy.env` → repo `.env`. Author: kejiqing
#[must_use]
pub fn resolve_deploy_env_file() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CLAW_DEPLOY_ENV_FILE") {
        let p = raw.trim();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    let mounted = PathBuf::from("/run/claw/deploy.env");
    if mounted.is_file() {
        return Some(mounted);
    }
    resolve_repo_env_file()
}

#[must_use]
pub fn resolve_bootstrap_repo_root() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CLAW_REPO_ROOT") {
        let p = raw.trim();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    resolve_deploy_env_file().and_then(|f| f.parent().map(Path::to_path_buf))
}

fn read_deploy_env_values(path: &Path) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(content) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line = trimmed.strip_prefix("export ").unwrap_or(trimmed);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !DEPLOY_ENV_WHITELIST.contains(&key) {
            continue;
        }
        let mut v = value.trim().to_string();
        if (v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"')) {
            v = v[1..v.len() - 1].to_string();
        }
        out.insert(key.to_string(), v);
    }
    out
}

pub fn bootstrap_env_snapshot(db: &GatewaySessionDb) -> BootstrapEnvSnapshot {
    let cluster_id =
        crate::cluster_identity::gateway_cluster_id().unwrap_or_else(|_| String::new());
    let deploy_path = resolve_deploy_env_file();
    let repo_root = resolve_bootstrap_repo_root();
    let mut values = deploy_path
        .as_deref()
        .map(read_deploy_env_values)
        .unwrap_or_default();
    for key in DEPLOY_ENV_WHITELIST {
        if !values.contains_key(*key) {
            if let Some(v) = env_trim(key) {
                if key.contains("KEY") || key.contains("TOKEN") || key.contains("PASSWORD") {
                    values.insert((*key).to_string(), "***".into());
                } else {
                    values.insert((*key).to_string(), v);
                }
            }
        }
    }
    let build_script = repo_root.as_ref().and_then(|r| {
        let publish = r.join("deploy/e2b/bootstrap-templates-from-ci-tag.sh");
        if publish.is_file() {
            return Some(publish);
        }
        let legacy = r.join("deploy/e2b/build-selfhosted-templates.sh");
        legacy.is_file().then_some(legacy)
    });
    let deploy_writable = deploy_path
        .as_ref()
        .and_then(|p| std::fs::OpenOptions::new().append(true).open(p).ok())
        .is_some();
    let pg_url_for_display = values
        .get("CLAW_GATEWAY_DATABASE_URL")
        .cloned()
        .or_else(|| env_trim("CLAW_GATEWAY_DATABASE_URL"));
    let pg_host_port = pg_url_for_display
        .as_deref()
        .and_then(|url| crate::cluster_identity::parse_pg_url(url).ok())
        .map(|p| format!("{}:{}", p.host, p.port));
    BootstrapEnvSnapshot {
        cluster_id,
        gateway_database_url: db.database_url_redacted().to_string(),
        deploy_profile: env_trim("CLAW_DEPLOY_PROFILE"),
        e2b_platform: gateway_e2b_platform_settings::e2b_platform_settings_public(),
        repo_root: repo_root.as_ref().map(|p| p.display().to_string()),
        deploy_env_path: deploy_path.as_ref().map(|p| p.display().to_string()),
        deploy_env_writable: deploy_writable,
        repo_root_present: repo_root.as_ref().is_some_and(|p| p.is_dir()),
        build_script_present: build_script.is_some(),
        pg_host_port,
        pg_rls_managed: true,
        values,
    }
}

async fn probe_pg_url(url: &str) -> bool {
    let url = url.trim();
    if url.is_empty() {
        return false;
    }
    match sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(url)
        .await
    {
        Ok(pool) => sqlx::query("SELECT 1").execute(&pool).await.is_ok(),
        Err(_) => false,
    }
}

async fn probe_e2b_api() -> bool {
    let Some(url) = env_trim("CLAW_E2B_API_URL") else {
        return false;
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return false;
    };
    let health = format!("{}/health", url.trim_end_matches('/'));
    client.get(health).send().await.is_ok()
}

/// Keys that bootstrap can hot-apply via process env + e2b client replace. Author: kejiqing
fn deploy_env_runtime_hot_key(key: &str) -> bool {
    matches!(
        key,
        "CLAW_E2B_API_URL"
            | "CLAW_E2B_SANDBOX_URL"
            | "CLAW_E2B_API_KEY"
            | "ALIYUN_E2B_TOKEN"
            | "CLAW_E2B_DOMAIN"
            | "CLAW_DEPLOY_PROFILE"
    )
}

/// Keys that identify which e2b *cluster* we talk to (not just credentials). Author: kejiqing
fn e2b_endpoint_key(key: &str) -> bool {
    matches!(
        key,
        "CLAW_E2B_API_URL" | "CLAW_E2B_SANDBOX_URL" | "CLAW_E2B_DOMAIN"
    )
}

/// Write whitelist keys, `set_var` so the running process sees them, then probe. Author: kejiqing
pub async fn apply_deploy_env(
    _db: &GatewaySessionDb,
    input: BootstrapApplyDeployEnvInput,
) -> Result<BootstrapApplyDeployEnvResponse, String> {
    let path = resolve_deploy_env_file().ok_or_else(|| {
        "deploy .env not found (mount /run/claw/deploy.env or set CLAW_REPO_ROOT)".to_string()
    })?;
    if !path.is_file() {
        std::fs::write(&path, "# claw gateway deploy env — bootstrap wizard\n")
            .map_err(|e| format!("create {}: {e}", path.display()))?;
    }
    let existing = read_deploy_env_values(&path);
    let mut applied = Vec::new();
    let mut applied_pairs: Vec<(String, String)> = Vec::new();
    let mut new_cluster_id: Option<String> = None;
    let mut e2b_endpoint_changed = false;
    for (key, value) in &input.values {
        let key = key.trim();
        if key.is_empty() || !DEPLOY_ENV_WHITELIST.contains(&key) {
            continue;
        }
        if key == "CLAW_GATEWAY_DATABASE_URL" {
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if key == "CLAW_CLUSTER_ID" {
            crate::cluster_identity::validate_cluster_id(value)?;
            let prev_env = env_trim(key);
            let prev = existing
                .get(key)
                .map(|s| s.as_str())
                .or(prev_env.as_deref());
            if prev != Some(value) {
                new_cluster_id = Some(value.to_string());
            }
        }
        if e2b_endpoint_key(key) {
            let prev_env = env_trim(key);
            let prev = existing
                .get(key)
                .map(|s| s.as_str())
                .or(prev_env.as_deref());
            if prev != Some(value) {
                e2b_endpoint_changed = true;
            }
        }
        upsert_dotenv_kv(&path, key, value)?;
        // Process env must match form so runtime replace sees the new .env. Author: kejiqing
        applied_pairs.push((key.to_string(), value.to_string()));
        if !applied.iter().any(|k| k == key) {
            applied.push(key.to_string());
        }
    }
    let mut synced_pg_url: Option<String> = None;
    if let Some(cluster_id) = new_cluster_id {
        let pg_base = existing
            .get("CLAW_GATEWAY_DATABASE_URL")
            .cloned()
            .or_else(|| env_trim("CLAW_GATEWAY_DATABASE_URL"))
            .ok_or_else(|| {
                "deploy .env 缺少 CLAW_GATEWAY_DATABASE_URL（运维预置共用 PG 连接，向导不填写）"
                    .to_string()
            })?;
        let synced = crate::cluster_identity::pg_url_with_rls_cluster_id(&pg_base, &cluster_id)?;
        upsert_dotenv_kv(&path, "CLAW_GATEWAY_DATABASE_URL", &synced)?;
        synced_pg_url = Some(synced.clone());
        applied_pairs.push(("CLAW_GATEWAY_DATABASE_URL".into(), synced));
        if !applied.iter().any(|k| k == "CLAW_GATEWAY_DATABASE_URL") {
            applied.push("CLAW_GATEWAY_DATABASE_URL".into());
        }
    }
    for (key, value) in &applied_pairs {
        // Bootstrap init: process must observe the same values as deploy .env. Author: kejiqing
        std::env::set_var(key, value);
    }
    // e2b / profile: runtime-replaceable. ClusterId / PG URL: session_db bound at connect. Author: kejiqing
    let restart_required = synced_pg_url.is_some()
        || applied
            .iter()
            .any(|k| !deploy_env_runtime_hot_key(k) && k != "CLAW_CLUSTER_ID");
    let validation = {
        let pg_url = synced_pg_url
            .or_else(|| env_trim("CLAW_GATEWAY_DATABASE_URL"))
            .unwrap_or_default();
        let pg_ok = probe_pg_url(&pg_url).await;
        let e2b_ok = probe_e2b_api().await;
        let message = match (pg_ok, e2b_ok) {
            (true, true) => None,
            (false, false) => Some("PostgreSQL and e2b API unreachable".into()),
            (false, true) => Some("PostgreSQL unreachable".into()),
            (true, false) => Some("e2b API unreachable".into()),
        };
        BootstrapEnvValidation {
            pg_ok,
            e2b_ok,
            message,
        }
    };
    Ok(BootstrapApplyDeployEnvResponse {
        applied,
        restart_required,
        env_file: path.display().to_string(),
        validation,
        templates_invalidated: e2b_endpoint_changed,
    })
}

/// Re-read e2b settings from process env into the live client (init / apply-deploy-env). Author: kejiqing
pub fn runtime_replace_e2b_from_env(
    client: &claw_e2b_sandbox_client::E2bSandboxClient,
) -> Result<(), String> {
    let next = claw_e2b_sandbox_client::E2bSandboxConfig::from_env().ok_or_else(|| {
        "CLAW_E2B_API_KEY (or ALIYUN_E2B_TOKEN) missing after apply; cannot replace e2b client"
            .to_string()
    })?;
    tracing::info!(
        target: "gateway_bootstrap",
        api_url = %next.api_url,
        domain = %next.domain,
        "e2b client runtime-replaced from process env"
    );
    client.reconfigure(next);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_includes_cluster_and_pg() {
        assert!(DEPLOY_ENV_WHITELIST.contains(&"CLAW_CLUSTER_ID"));
        assert!(DEPLOY_ENV_WHITELIST.contains(&"CLAW_GATEWAY_DATABASE_URL"));
    }
}
