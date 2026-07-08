//! e2b observe singleton template id in PG (`settings_json.e2bObserve`). Author: kejiqing

use serde::{Deserialize, Serialize};

use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;
use claw_e2b_sandbox_client::E2bSandboxClient;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct E2bObserveSettings {
    #[serde(rename = "templateId", default)]
    pub template_id: Option<String>,
    #[serde(rename = "updatedAtMs", default)]
    pub updated_at_ms: i64,
}

impl E2bObserveSettings {
    #[must_use]
    pub fn configured(&self) -> bool {
        self.template_id
            .as_ref()
            .is_some_and(|t| !t.trim().is_empty())
    }
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize)]
pub struct E2bObserveSettingsPublic {
    #[serde(rename = "templateId", skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(rename = "effectiveTemplateId")]
    pub effective_template_id: String,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    pub configured: bool,
    #[serde(rename = "baseUrl", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(rename = "sandboxId", skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub running: Option<bool>,
    pub reachable: bool,
    pub healthy: bool,
    #[serde(rename = "lastCheckedAtMs", skip_serializing_if = "Option::is_none")]
    pub last_checked_at_ms: Option<i64>,
    #[serde(rename = "lastError", skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// PG `e2bObserve.templateId` → env `CLAW_E2B_OBSERVE_TEMPLATE` → `claw-observe`.
#[must_use]
pub fn e2b_observe_template_from_env() -> String {
    std::env::var("CLAW_E2B_OBSERVE_TEMPLATE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claw-observe".into())
}

pub async fn load_e2b_observe_settings(
    db: &GatewaySessionDb,
) -> Result<E2bObserveSettings, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings.e2b_observe)
}

pub async fn load_e2b_observe_template_id(db: &GatewaySessionDb) -> Result<String, sqlx::Error> {
    let settings = load_e2b_observe_settings(db).await?;
    Ok(settings
        .template_id
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(e2b_observe_template_from_env))
}

pub async fn e2b_observe_settings_public(
    db: &GatewaySessionDb,
) -> Result<E2bObserveSettingsPublic, sqlx::Error> {
    e2b_observe_settings_public_with_runtime(db, None).await
}

pub async fn e2b_observe_settings_public_with_runtime(
    db: &GatewaySessionDb,
    client: Option<&E2bSandboxClient>,
) -> Result<E2bObserveSettingsPublic, sqlx::Error> {
    let settings = load_e2b_observe_settings(db).await?;
    let (global, _, _) = get_gateway_global_settings(db).await?;
    let effective_template_id = load_e2b_observe_template_id(db).await?;
    let configured = !effective_template_id.trim().is_empty();
    let base_url = global
        .claw_tap
        .live_base_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_string);
    let sandbox_id = global
        .claw_tap
        .e2b_observe_sandbox_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string);

    let mut last_error: Option<String> = None;
    let mut running: Option<bool> = None;
    if let (Some(c), Some(sid)) = (client, sandbox_id.as_deref()) {
        match c.fetch_sandbox_snapshot(sid).await {
            Ok(snap) => {
                let is_running = snap.is_running();
                running = Some(is_running);
                if !is_running {
                    last_error = Some(format!("sandbox {sid} not running (state={})", snap.state));
                }
            }
            Err(e) => {
                last_error = Some(format!("sandbox status check failed: {e}"));
            }
        }
    }

    let (reachable, health_error) = match base_url.as_deref() {
        Some(base) => match reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(3))
            .timeout(std::time::Duration::from_secs(8))
            .build()
        {
            Ok(http) => match http.get(base).send().await {
                Ok(resp) if resp.status().is_success() => (true, None),
                Ok(resp) => (false, Some(format!("observe Live HTTP {}", resp.status()))),
                Err(e) => (false, Some(format!("observe Live request failed: {e}"))),
            },
            Err(e) => (false, Some(format!("health client build failed: {e}"))),
        },
        None => (
            false,
            Some("observe liveBaseUrl not configured".to_string()),
        ),
    };
    if last_error.is_none() {
        last_error = health_error;
    }
    let healthy = reachable && running.unwrap_or(true);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok());
    Ok(E2bObserveSettingsPublic {
        template_id: settings.template_id,
        effective_template_id,
        updated_at_ms: settings.updated_at_ms,
        configured,
        base_url,
        sandbox_id,
        running,
        reachable,
        healthy,
        last_checked_at_ms: now_ms,
        last_error: if healthy { None } else { last_error },
    })
}
