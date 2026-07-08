//! Admin API for e2b core singletons (nas-api / observe / ovs). Author: kejiqing

use claw_e2b_sandbox_client::E2bSandboxClient;
use serde::{Deserialize, Serialize};

use crate::gateway_e2b_nas_api_settings::{
    e2b_nas_api_settings_public, e2b_nas_api_settings_public_with_runtime, E2bNasApiSettingsPublic,
};
use crate::gateway_e2b_observe_settings::{
    e2b_observe_settings_public, e2b_observe_settings_public_with_runtime, E2bObserveSettingsPublic,
};
use crate::gateway_e2b_ovs_settings::{e2b_ovs_settings_public, E2bOvsSettingsPublic};
use crate::gateway_e2b_singleton_lifecycle::{
    ensure_e2b_singleton, reset_e2b_singleton, E2bSingletonComponent,
};
use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Serialize)]
pub struct E2bSingletonsStatusResponse {
    #[serde(rename = "nasApi")]
    pub nas_api: E2bNasApiSettingsPublic,
    pub ovs: E2bOvsSettingsPublic,
    pub observe: E2bObserveSettingsPublic,
}

#[derive(Debug, Deserialize)]
pub struct PutE2bSingletonTemplatesInput {
    #[serde(default, rename = "nasApiTemplateId")]
    pub nas_api_template_id: Option<String>,
    #[serde(default, rename = "ovsTemplateId")]
    pub ovs_template_id: Option<String>,
    #[serde(default, rename = "observeTemplateId")]
    pub observe_template_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PutE2bSingletonTemplatesResponse {
    #[serde(rename = "nasApi")]
    pub nas_api: E2bNasApiSettingsPublic,
    pub ovs: E2bOvsSettingsPublic,
    pub observe: E2bObserveSettingsPublic,
}

#[derive(Debug, Serialize)]
pub struct E2bSingletonActionResponse {
    pub component: String,
    #[serde(rename = "sandboxId", skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    #[serde(rename = "baseUrl", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(rename = "trafficReachable")]
    pub traffic_reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "nasApi", skip_serializing_if = "Option::is_none")]
    pub nas_api: Option<E2bNasApiSettingsPublic>,
    #[serde(rename = "e2bOvs", skip_serializing_if = "Option::is_none")]
    pub e2b_ovs: Option<E2bOvsSettingsPublic>,
    pub observe: Option<E2bObserveSettingsPublic>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0)
}

fn normalize_template_id(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[derive(Debug, Serialize)]
pub struct E2bTemplatesListResponse {
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    pub templates: Vec<claw_e2b_sandbox_client::E2bTemplateEntry>,
}

pub async fn list_e2b_templates(client: &E2bSandboxClient) -> Result<E2bTemplatesListResponse, String> {
    let templates = client.list_templates().await?;
    Ok(E2bTemplatesListResponse {
        api_url: client.config().api_url.clone(),
        templates,
    })
}

pub async fn load_e2b_singletons_status(
    db: &GatewaySessionDb,
    client: Option<&E2bSandboxClient>,
) -> Result<E2bSingletonsStatusResponse, sqlx::Error> {
    Ok(E2bSingletonsStatusResponse {
        nas_api: match client {
            Some(c) => e2b_nas_api_settings_public_with_runtime(db, Some(c)).await?,
            None => e2b_nas_api_settings_public(db).await?,
        },
        ovs: e2b_ovs_settings_public(db).await?,
        observe: match client {
            Some(c) => e2b_observe_settings_public_with_runtime(db, Some(c)).await?,
            None => e2b_observe_settings_public(db).await?,
        },
    })
}

pub async fn put_e2b_singleton_templates(
    db: &GatewaySessionDb,
    input: PutE2bSingletonTemplatesInput,
) -> Result<PutE2bSingletonTemplatesResponse, String> {
    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    let now = now_ms();
    let mut changed = false;

    if let Some(tid) = normalize_template_id(input.nas_api_template_id) {
        settings.e2b_nas_api.template_id = Some(tid);
        settings.e2b_nas_api.updated_at_ms = now;
        changed = true;
    }
    if let Some(tid) = normalize_template_id(input.ovs_template_id) {
        settings.e2b_ovs.template_id = Some(tid);
        settings.e2b_ovs.updated_at_ms = now;
        changed = true;
    }
    if let Some(tid) = normalize_template_id(input.observe_template_id) {
        settings.e2b_observe.template_id = Some(tid);
        settings.e2b_observe.updated_at_ms = now;
        changed = true;
    }

    if !changed {
        return Err("no templateId fields provided".into());
    }

    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;

    Ok(PutE2bSingletonTemplatesResponse {
        nas_api: e2b_nas_api_settings_public(db)
            .await
            .map_err(|e| e.to_string())?,
        ovs: e2b_ovs_settings_public(db)
            .await
            .map_err(|e| e.to_string())?,
        observe: e2b_observe_settings_public(db)
            .await
            .map_err(|e| e.to_string())?,
    })
}

pub fn parse_singleton_component(raw: &str) -> Result<E2bSingletonComponent, String> {
    E2bSingletonComponent::parse(raw)
}

pub async fn ensure_e2b_singleton_via_api(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    component: E2bSingletonComponent,
) -> Result<E2bSingletonActionResponse, String> {
    let outcome = ensure_e2b_singleton(db, client, component).await?;
    build_action_response(db, client, component, outcome).await
}

pub async fn reset_e2b_singleton_via_api(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    component: E2bSingletonComponent,
) -> Result<E2bSingletonActionResponse, String> {
    let outcome = reset_e2b_singleton(db, client, component).await?;
    build_action_response(db, client, component, outcome).await
}

async fn build_action_response(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    component: E2bSingletonComponent,
    outcome: crate::gateway_e2b_singleton_lifecycle::E2bSingletonOutcome,
) -> Result<E2bSingletonActionResponse, String> {
    let status = load_e2b_singletons_status(db, Some(client))
        .await
        .map_err(|e| e.to_string())?;
    Ok(E2bSingletonActionResponse {
        component: component.as_str().to_string(),
        sandbox_id: outcome.sandbox_id,
        base_url: outcome.base_url,
        traffic_reachable: outcome.traffic_reachable,
        message: outcome.message,
        nas_api: (component == E2bSingletonComponent::NasApi).then(|| status.nas_api.clone()),
        e2b_ovs: (component == E2bSingletonComponent::Ovs).then(|| status.ovs.clone()),
        observe: (component == E2bSingletonComponent::Observe).then(|| status.observe.clone()),
    })
}
