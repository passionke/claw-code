//! Admin API for per-project e2b warm worker status + force reset. Author: kejiqing

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle};
use serde::Serialize;

use crate::gateway_e2b_observe_proxy;
use crate::gateway_e2b_worker_settings::{
    load_e2b_strict_worker_pool_size, load_e2b_worker_relaxed_template_id,
    load_e2b_worker_template_id,
};
use crate::pool::interactive_backend::{terminal_ws_connect_url, TtydConnectTarget};
use crate::pool::{
    default_worker_profile_json, effective_mode, profile_mode_label,
    relaxed_worker_allowed_from_env, E2bProjWorkerRegistry, WorkerProfileMode,
};
use crate::session_db::{
    e2b_worker_slot_u32, GatewaySessionDb, ProjectFcWorkerRow, WorkerRotationEvent,
};

#[derive(Debug, Clone, Serialize)]
pub struct ProjectE2bWorkerUrls {
    #[serde(rename = "e2bApiUrl")]
    pub e2b_api_url: String,
    #[serde(rename = "trafficProxyBase", skip_serializing_if = "Option::is_none")]
    pub traffic_proxy_base: Option<String>,
    #[serde(rename = "sandboxDomain")]
    pub sandbox_domain: String,
    #[serde(rename = "ttydPublicHost", skip_serializing_if = "Option::is_none")]
    pub ttyd_public_host: Option<String>,
    #[serde(rename = "ttydWsUrl", skip_serializing_if = "Option::is_none")]
    pub ttyd_ws_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectE2bWorkerInfo {
    #[serde(rename = "slotIndex")]
    pub slot_index: i32,
    #[serde(rename = "activeLeases", skip_serializing_if = "Option::is_none")]
    pub active_leases: Option<u32>,
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
    #[serde(rename = "workerProfile")]
    pub worker_profile: String,
    #[serde(rename = "desiredTemplate")]
    pub desired_template: String,
    #[serde(rename = "desiredPoolSize")]
    pub desired_pool_size: u32,
    pub workers: Vec<ProjectE2bWorkerInfo>,
    #[serde(rename = "rotationLog")]
    pub rotation_log: Vec<WorkerRotationEventPublic>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectE2bWorkerResetResponse {
    #[serde(rename = "projId")]
    pub proj_id: i64,
    pub ok: bool,
    pub workers: Vec<ProjectE2bWorkerInfo>,
    #[serde(rename = "rotationLog")]
    pub rotation_log: Vec<WorkerRotationEventPublic>,
}

fn worker_urls_strict(
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
) -> ProjectE2bWorkerUrls {
    let cfg = client.config();
    ProjectE2bWorkerUrls {
        e2b_api_url: cfg.api_url.clone(),
        traffic_proxy_base: cfg.sandbox_url.clone(),
        sandbox_domain: handle.sandbox_domain.clone(),
        ttyd_public_host: None,
        ttyd_ws_url: None,
    }
}

fn worker_urls_relaxed(
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
) -> ProjectE2bWorkerUrls {
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
        ttyd_public_host: Some(handle.ttyd_public_host.clone()),
        ttyd_ws_url: Some(terminal_ws_connect_url(&ttyd_target)),
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
    registry: &E2bProjWorkerRegistry,
    client: &E2bSandboxClient,
    row: &ProjectFcWorkerRow,
    relaxed: bool,
) -> Result<ProjectE2bWorkerInfo, String> {
    let handle = E2bSandboxClient::handle_from_json(&row.handle_json)?;
    let snap = client.fetch_sandbox_snapshot(&row.sandbox_id).await.ok();
    let running = snap.as_ref().is_some_and(|s| s.is_running());
    let now_ms = chrono::Utc::now().timestamp_millis();
    let remaining_ttl_secs = snap.and_then(|s| s.remaining_ttl_secs(now_ms));
    let active_leases = registry
        .active_leases_for_slot(row.proj_id, e2b_worker_slot_u32(row.slot_index))
        .await;
    let urls = if relaxed {
        worker_urls_relaxed(client, &handle)
    } else {
        worker_urls_strict(client, &handle)
    };
    Ok(ProjectE2bWorkerInfo {
        slot_index: row.slot_index,
        active_leases: Some(active_leases),
        sandbox_id: row.sandbox_id.clone(),
        worker_id: row.worker_id.clone(),
        template_contract: row.template_id.clone(),
        running,
        remaining_ttl_secs,
        updated_at_ms: row.updated_at_ms,
        urls,
    })
}

async fn project_profile_mode(db: &GatewaySessionDb, proj_id: i64) -> WorkerProfileMode {
    let json = db
        .get_worker_profile_json(proj_id)
        .await
        .unwrap_or_else(|_| default_worker_profile_json());
    effective_mode(relaxed_worker_allowed_from_env(), &json)
}

pub async fn get_project_e2b_worker_status(
    registry: &E2bProjWorkerRegistry,
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    proj_id: i64,
) -> Result<ProjectE2bWorkerStatusResponse, String> {
    let mode = project_profile_mode(db, proj_id).await;
    let relaxed = mode == WorkerProfileMode::Relaxed;
    let profile_json = db
        .get_worker_profile_json(proj_id)
        .await
        .unwrap_or_else(|_| default_worker_profile_json());
    let worker_profile = profile_mode_label(&profile_json).to_string();
    let desired_template = if relaxed {
        load_e2b_worker_relaxed_template_id(db)
            .await
            .map_err(|e| format!("load e2bWorkerRelaxed template: {e}"))?
    } else {
        load_e2b_worker_template_id(db)
            .await
            .map_err(|e| format!("load e2bWorker template: {e}"))?
    };
    let desired_pool_size = if relaxed {
        1
    } else {
        load_e2b_strict_worker_pool_size(db)
            .await
            .map_err(|e| format!("load poolSize: {e}"))?
    };
    let rows = db
        .list_project_e2b_workers(proj_id)
        .await
        .map_err(|e| format!("list project_e2b_workers: {e}"))?;
    let mut workers = Vec::new();
    for row in &rows {
        workers.push(build_worker_info(registry, client, row, relaxed).await?);
    }
    let rotation_log = load_rotation_log(db, proj_id).await?;
    Ok(ProjectE2bWorkerStatusResponse {
        proj_id,
        worker_profile,
        desired_template,
        desired_pool_size,
        workers,
        rotation_log,
    })
}

pub async fn reset_project_e2b_worker(
    registry: &E2bProjWorkerRegistry,
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    proj_id: i64,
    slot_index: Option<u32>,
) -> Result<ProjectE2bWorkerResetResponse, String> {
    registry.force_rotate_proj(proj_id, slot_index).await?;
    let status = get_project_e2b_worker_status(registry, db, client, proj_id).await?;
    if status.workers.is_empty() {
        return Err(format!(
            "proj worker missing after force reset proj_{proj_id}"
        ));
    }
    Ok(ProjectE2bWorkerResetResponse {
        proj_id,
        ok: true,
        workers: status.workers,
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
