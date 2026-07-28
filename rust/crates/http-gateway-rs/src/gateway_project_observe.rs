//! Per-project observe singleton lifecycle (only when project LLM override is active).
//! Author: kejiqing

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle};
use serde::Serialize;
use tracing::{info, warn};

use crate::cluster_identity::{gateway_cluster_id, sandbox_database_url};
use crate::gateway_claw_tap_settings::{DEFAULT_CLAW_TAP_LIVE_PORT, DEFAULT_CLAW_TAP_PROXY_PORT};
use crate::gateway_e2b_observe_settings::load_e2b_observe_template_id;
use crate::gateway_llm_cluster_store::resolve_llm_cluster_id;
use crate::gateway_project_llm::{
    load_active_project_llm_runtime, load_project_inference_settings, ProjectObservePublic,
};
use crate::pool::interactive_backend::e2b_observe_is_enabled;
use crate::session_db::{GatewayLlmProjectObserveRow, GatewaySessionDb};

#[derive(Debug, Clone, Serialize)]
pub struct ProjectObserveStatusResponse {
    #[serde(rename = "projId")]
    pub proj_id: i64,
    pub observe: ProjectObservePublic,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectObserveResetResponse {
    #[serde(rename = "projId")]
    pub proj_id: i64,
    pub observe: ProjectObservePublic,
    #[serde(rename = "sandboxId", skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn observe_live_port() -> u16 {
    std::env::var("CLAW_E2B_OBSERVE_LIVE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CLAW_TAP_LIVE_PORT)
}

fn service_base_url(
    client: &E2bSandboxClient,
    port: u16,
    sandbox_id: &str,
    domain: &str,
) -> String {
    let host = client.service_public_host(port, sandbox_id, domain);
    let scheme = if client.config().is_self_hosted() {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{host}")
}

async fn wait_http_ok(url: &str, label: &str, max_secs: u64) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()
    else {
        return false;
    };
    for _ in 0..max_secs {
        if let Ok(resp) = client.get(url).send().await {
            if resp.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    warn!(target: "claw_project_observe", %label, %url, "wait_http_ok timed out");
    false
}

async fn persist_project_observe(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    cluster_id: &str,
    proj_id: i64,
    handle: &E2bSandboxHandle,
    live_port: u16,
    live_base: &str,
) -> Result<(), sqlx::Error> {
    let proxy_host = client.service_public_host(
        DEFAULT_CLAW_TAP_PROXY_PORT,
        &handle.sandbox_id,
        &handle.sandbox_domain,
    );
    let proxy_base = service_base_url(
        client,
        DEFAULT_CLAW_TAP_PROXY_PORT,
        &handle.sandbox_id,
        &handle.sandbox_domain,
    );
    db.save_llm_project_observe(&GatewayLlmProjectObserveRow {
        cluster_id: cluster_id.to_string(),
        proj_id,
        sandbox_id: handle.sandbox_id.clone(),
        proxy_base_url: proxy_base,
        live_base_url: live_base.to_string(),
        host: proxy_host,
        proxy_port: i32::from(DEFAULT_CLAW_TAP_PROXY_PORT),
        live_port: i32::from(live_port),
        updated_at_ms: now_ms(),
    })
    .await
}

async fn resolve_project_observe_sandbox_id(
    client: &E2bSandboxClient,
    cluster_id: &str,
    proj_id: i64,
    pg_sandbox_id: Option<&str>,
) -> Option<String> {
    if let Some(pg_id) = pg_sandbox_id.map(str::trim).filter(|s| !s.is_empty()) {
        if client.sandbox_running(pg_id).await {
            return Some(pg_id.to_string());
        }
    }
    client
        .find_observe_proj(cluster_id, proj_id)
        .await
        .ok()
        .flatten()
}

/// Ensure project observe when override LLM is active. No-op (and teardown) when inherit.
/// Author: kejiqing
pub async fn ensure_project_observe(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    proj_id: i64,
) -> Result<ProjectObserveResetResponse, String> {
    if !e2b_observe_is_enabled() {
        return Ok(ProjectObserveResetResponse {
            proj_id,
            observe: ProjectObservePublic {
                configured: false,
                sandbox_id: None,
                proxy_base_url: None,
                live_base_url: None,
                host: None,
                proxy_port: None,
                live_port: None,
                updated_at_ms: None,
                e2b_observe_sandbox_running: None,
            },
            sandbox_id: None,
            message: Some("observe disabled".into()),
        });
    }

    let active = load_active_project_llm_runtime(db, proj_id)
        .await
        .map_err(|e| e.to_string())?;
    if active.is_none() {
        teardown_project_observe(db, client, proj_id).await?;
        let settings = load_project_inference_settings(db, proj_id).await?;
        return Ok(ProjectObserveResetResponse {
            proj_id,
            observe: settings.observe,
            sandbox_id: None,
            message: Some("project inherits global LLM; project observe torn down".into()),
        });
    }

    let cluster_id = gateway_cluster_id()?;
    let template = load_e2b_observe_template_id(db)
        .await
        .map_err(|e| format!("load observe template: {e}"))?;
    let sandbox_db_url = sandbox_database_url()?;
    let live_port = observe_live_port();
    let pg_row = db
        .get_llm_project_observe(&cluster_id, proj_id)
        .await
        .map_err(|e| e.to_string())?;
    let pg_sid = pg_row.as_ref().map(|r| r.sandbox_id.as_str());

    let candidate =
        resolve_project_observe_sandbox_id(client, &cluster_id, proj_id, pg_sid).await;

    if let Some(ref sid) = candidate {
        let domain = client.config().domain.clone();
        let live_base = service_base_url(client, live_port, sid, &domain);
        if client.sandbox_running(sid).await && wait_http_ok(&live_base, "observe-proj Live", 3).await
        {
            client.touch_persistent_sandbox(sid).await?;
            let handle = E2bSandboxHandle {
                sandbox_id: sid.clone(),
                sandbox_domain: domain,
                envd_access_token: None,
                traffic_access_token: None,
                ttyd_public_host: String::new(),
                ttyd_use_tls: !client.config().is_self_hosted(),
                ovs_public_host: None,
                ovs_base_url: None,
            };
            persist_project_observe(db, client, &cluster_id, proj_id, &handle, live_port, &live_base)
                .await
                .map_err(|e| e.to_string())?;
            info!(target: "claw_project_observe", proj_id, sandbox_id = %sid, "project observe online");
            let settings = load_project_inference_settings(db, proj_id).await?;
            return Ok(ProjectObserveResetResponse {
                proj_id,
                observe: settings.observe,
                sandbox_id: Some(sid.clone()),
                message: None,
            });
        }
        warn!(
            target: "claw_project_observe",
            proj_id,
            sandbox_id = %sid,
            "project observe unhealthy — recreate"
        );
        let _ = client.kill_sandbox(sid).await;
    }

    info!(
        target: "claw_project_observe",
        template = %template,
        cluster_id = %cluster_id,
        proj_id,
        "create project observe"
    );
    let handle = client
        .create_observe_proj_singleton(&template, &cluster_id, proj_id, &sandbox_db_url)
        .await?;
    let live_base = service_base_url(
        client,
        live_port,
        &handle.sandbox_id,
        &handle.sandbox_domain,
    );
    if !wait_http_ok(&live_base, "observe-proj Live", 60).await {
        let _ = client.kill_sandbox(&handle.sandbox_id).await;
        return Err(format!("project observe Live not reachable at {live_base}"));
    }
    persist_project_observe(
        db,
        client,
        &cluster_id,
        proj_id,
        &handle,
        live_port,
        &live_base,
    )
    .await
    .map_err(|e| e.to_string())?;
    client.track_persistent_sandbox(&handle.sandbox_id);
    let settings = load_project_inference_settings(db, proj_id).await?;
    Ok(ProjectObserveResetResponse {
        proj_id,
        observe: settings.observe,
        sandbox_id: Some(handle.sandbox_id),
        message: None,
    })
}

/// Kill then ensure project observe (Admin reset). Author: kejiqing
pub async fn reset_project_observe(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    proj_id: i64,
) -> Result<ProjectObserveResetResponse, String> {
    let cluster_id = gateway_cluster_id()?;
    let pg_row = db
        .get_llm_project_observe(&cluster_id, proj_id)
        .await
        .map_err(|e| e.to_string())?;
    let pg_sid = pg_row.as_ref().map(|r| r.sandbox_id.as_str());
    if let Some(sid) =
        resolve_project_observe_sandbox_id(client, &cluster_id, proj_id, pg_sid).await
    {
        info!(
            target: "claw_project_observe",
            proj_id,
            sandbox_id = %sid,
            "kill project observe before reset"
        );
        let _ = client.kill_sandbox(&sid).await;
    }
    let _ = db
        .delete_llm_project_observe(&cluster_id, proj_id)
        .await
        .map_err(|e| e.to_string())?;
    ensure_project_observe(db, client, proj_id).await
}

/// Tear down project observe (when falling back to global LLM). Author: kejiqing
pub async fn teardown_project_observe(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    proj_id: i64,
) -> Result<(), String> {
    let Some(cluster_id) = resolve_llm_cluster_id() else {
        return Ok(());
    };
    let pg_row = db
        .get_llm_project_observe(&cluster_id, proj_id)
        .await
        .map_err(|e| e.to_string())?;
    let pg_sid = pg_row.as_ref().map(|r| r.sandbox_id.as_str());
    if let Some(sid) =
        resolve_project_observe_sandbox_id(client, &cluster_id, proj_id, pg_sid).await
    {
        info!(
            target: "claw_project_observe",
            proj_id,
            sandbox_id = %sid,
            "teardown project observe"
        );
        let _ = client.kill_sandbox(&sid).await;
    }
    db.delete_llm_project_observe(&cluster_id, proj_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn get_project_observe_status(
    db: &GatewaySessionDb,
    client: Option<&E2bSandboxClient>,
    proj_id: i64,
) -> Result<ProjectObserveStatusResponse, String> {
    let mut settings = load_project_inference_settings(db, proj_id).await?;
    if let (Some(client), Some(sid)) = (client, settings.observe.sandbox_id.clone()) {
        settings.observe.e2b_observe_sandbox_running =
            Some(client.sandbox_running(&sid).await);
    }
    Ok(ProjectObserveStatusResponse {
        proj_id,
        observe: settings.observe,
        message: None,
    })
}

/// Persist a mock observe row (tests / dry-run without e2b). Author: kejiqing
pub async fn persist_project_observe_urls_for_test(
    db: &GatewaySessionDb,
    cluster_id: &str,
    proj_id: i64,
    sandbox_id: &str,
    proxy_base_url: &str,
) -> Result<(), String> {
    db.save_llm_project_observe(&GatewayLlmProjectObserveRow {
        cluster_id: cluster_id.to_string(),
        proj_id,
        sandbox_id: sandbox_id.to_string(),
        proxy_base_url: proxy_base_url.to_string(),
        live_base_url: format!("{proxy_base_url}-live"),
        host: "test-observe".into(),
        proxy_port: i32::from(DEFAULT_CLAW_TAP_PROXY_PORT),
        live_port: i32::from(DEFAULT_CLAW_TAP_LIVE_PORT),
        updated_at_ms: now_ms(),
    })
    .await
    .map_err(|e| e.to_string())
}
