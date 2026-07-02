//! Admin API for per-project e2b warm worker status + force reset. Author: kejiqing

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle};
use serde::Serialize;

use crate::gateway_e2b_observe_proxy;
use crate::gateway_e2b_worker_settings::load_e2b_worker_template_id;
use crate::pool::interactive_backend::{terminal_ws_connect_url, TtydConnectTarget};
use crate::pool::E2bProjWorkerRegistry;
use crate::session_db::{GatewaySessionDb, WorkerRotationEvent};

#[derive(Debug, Clone, Serialize)]
pub struct ProjectE2bWorkerUrls {
    #[serde(rename = "e2bApiUrl")]
    pub e2b_api_url: String,
    #[serde(rename = "trafficProxyBase", skip_serializing_if = "Option::is_none")]
    pub traffic_proxy_base: Option<String>,
    #[serde(rename = "sandboxDomain")]
    pub sandbox_domain: String,
    #[serde(rename = "ttydPublicHost")]
    pub ttyd_public_host: String,
    #[serde(rename = "ttydWsUrl")]
    pub ttyd_ws_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectE2bWorkerInfo {
    #[serde(rename = "sandboxId")]
    pub sandbox_id: String,
    #[serde(rename = "workerId")]
    pub worker_id: String,
    #[serde(rename = "templateContract")]
    pub template_contract: String,
    pub running: bool,
    #[serde(rename = "remainingTtlSecs", skip_serializing_if = "Option::is_none")]
    pub remaining_ttl_secs: Option<u64>,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: i64,
    pub urls: ProjectE2bWorkerUrls,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerRotationEventPublic {
    pub event: String,
    #[serde(rename = "sandboxId", skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    #[serde(rename = "workerId", skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(rename = "templateId", skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "atMs")]
    pub at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectE2bWorkerStatusResponse {
    #[serde(rename = "projId")]
    pub proj_id: i64,
    #[serde(rename = "desiredTemplate")]
    pub desired_template: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker: Option<ProjectE2bWorkerInfo>,
    #[serde(rename = "rotationLog")]
    pub rotation_log: Vec<WorkerRotationEventPublic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectE2bWorkerResetResponse {
    #[serde(rename = "projId")]
    pub proj_id: i64,
    pub ok: bool,
    pub worker: ProjectE2bWorkerInfo,
    #[serde(rename = "rotationLog")]
    pub rotation_log: Vec<WorkerRotationEventPublic>,
}

fn worker_urls(client: &E2bSandboxClient, handle: &E2bSandboxHandle) -> ProjectE2bWorkerUrls {
    let cfg = client.config();
    let ttyd_target = if handle.ttyd_use_tls {
        TtydConnectTarget::e2b_public(handle.ttyd_public_host.clone())
    } else {
        let traffic_host = cfg
            .sandbox_url
            .as_deref()
            .and_then(parse_proxy_host)
            .unwrap_or_else(|| cfg.domain.clone());
        TtydConnectTarget::e2b_self_hosted_proxy(
            traffic_host,
            gateway_e2b_observe_proxy::e2b_traffic_proxy_port(),
            handle.ttyd_public_host.clone(),
            handle.traffic_access_token.clone(),
        )
    };
    ProjectE2bWorkerUrls {
        e2b_api_url: cfg.api_url.clone(),
        traffic_proxy_base: cfg.sandbox_url.clone(),
        sandbox_domain: handle.sandbox_domain.clone(),
        ttyd_public_host: handle.ttyd_public_host.clone(),
        ttyd_ws_url: terminal_ws_connect_url(&ttyd_target),
    }
}

fn parse_proxy_host(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let no_scheme = trimmed
        .strip_prefix("http://")
        .or_else(|| trimmed.strip_prefix("https://"))?;
    let host = no_scheme.split('/').next()?.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn rotation_event_public(e: WorkerRotationEvent) -> WorkerRotationEventPublic {
    WorkerRotationEventPublic {
        event: e.event,
        sandbox_id: e.sandbox_id,
        worker_id: e.worker_id,
        template_id: e.template_id,
        reason: e.reason,
        at_ms: e.at_ms,
    }
}

async fn build_worker_info(
    client: &E2bSandboxClient,
    sandbox_id: &str,
    worker_id: &str,
    template_contract: &str,
    handle: &E2bSandboxHandle,
    updated_at_ms: i64,
) -> Result<ProjectE2bWorkerInfo, String> {
    let snap = client.fetch_sandbox_snapshot(sandbox_id).await.ok();
    let running = snap.as_ref().is_some_and(|s| s.is_running());
    let now_ms = chrono::Utc::now().timestamp_millis();
    let remaining_ttl_secs = snap.and_then(|s| s.remaining_ttl_secs(now_ms));
    Ok(ProjectE2bWorkerInfo {
        sandbox_id: sandbox_id.to_string(),
        worker_id: worker_id.to_string(),
        template_contract: template_contract.to_string(),
        running,
        remaining_ttl_secs,
        updated_at_ms,
        urls: worker_urls(client, handle),
    })
}

pub async fn get_project_e2b_worker_status(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    proj_id: i64,
) -> Result<ProjectE2bWorkerStatusResponse, String> {
    let desired_template = load_e2b_worker_template_id(db)
        .await
        .map_err(|e| format!("load e2bWorker template: {e}"))?;
    let row = db
        .get_project_e2b_worker(proj_id)
        .await
        .map_err(|e| format!("get project_e2b_worker: {e}"))?;
    let worker = if let Some(ref existing) = row {
        let handle = E2bSandboxClient::handle_from_json(&existing.handle_json)?;
        Some(
            build_worker_info(
                client,
                &existing.sandbox_id,
                &existing.worker_id,
                &existing.template_id,
                &handle,
                existing.updated_at_ms,
            )
            .await?,
        )
    } else {
        None
    };
    let rotation_log = load_rotation_log(db, proj_id).await?;
    Ok(ProjectE2bWorkerStatusResponse {
        proj_id,
        desired_template,
        worker,
        rotation_log,
    })
}

pub async fn reset_project_e2b_worker(
    registry: &E2bProjWorkerRegistry,
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    proj_id: i64,
) -> Result<ProjectE2bWorkerResetResponse, String> {
    registry.force_rotate_proj(proj_id).await?;
    let status = get_project_e2b_worker_status(db, client, proj_id).await?;
    let worker = status
        .worker
        .ok_or_else(|| format!("proj worker missing after force reset proj_{proj_id}"))?;
    Ok(ProjectE2bWorkerResetResponse {
        proj_id,
        ok: true,
        worker,
        rotation_log: status.rotation_log,
    })
}

async fn load_rotation_log(
    db: &GatewaySessionDb,
    proj_id: i64,
) -> Result<Vec<WorkerRotationEventPublic>, String> {
    let rows = db
        .list_worker_rotation_log(proj_id, 20)
        .await
        .map_err(|e| format!("list worker_rotation_log: {e}"))?;
    Ok(rows.into_iter().map(rotation_event_public).collect())
}
