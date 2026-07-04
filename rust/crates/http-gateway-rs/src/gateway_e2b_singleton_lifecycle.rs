//! e2b singleton ensure (observe / nas-api / ovs) + lease ticker registration. Author: kejiqing

use std::sync::Arc;
use std::time::Duration;

use claw_e2b_sandbox_client::{
    E2bSandboxClient, E2bSandboxHandle, SINGLETON_ROLE_NAS_API, SINGLETON_ROLE_OBSERVE,
    SINGLETON_ROLE_OVS,
};
use tracing::{info, warn};

use crate::cluster_identity::{
    fetch_tap_cluster_identity, gateway_cluster_id, sandbox_database_url,
};
use crate::gateway_claw_tap_settings::{
    live_session_viewer_url_template, ClawTapMode, ClawTapSettings, DEFAULT_CLAW_TAP_LIVE_PORT,
    DEFAULT_CLAW_TAP_PROXY_PORT,
};
use crate::gateway_e2b_nas_api_settings::{load_e2b_nas_api_template_id, E2bNasApiSettings};
use crate::gateway_e2b_observe_settings::load_e2b_observe_template_id;
use crate::gateway_e2b_ovs_settings::{load_e2b_ovs_template_id, E2bOvsSettings};
use crate::gateway_e2b_worker_settings::e2b_project_worker_renew_interval_secs_from_env;
use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::pool::interactive_backend::{
    e2b_observe_is_enabled, interactive_backend_is_e2b, E2bNasApiSingleton,
};
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum E2bSingletonComponent {
    NasApi,
    Observe,
    Ovs,
}

impl E2bSingletonComponent {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NasApi => "nas-api",
            Self::Observe => "observe",
            Self::Ovs => "ovs",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "nas-api" | "nas_api" | "nasapi" => Ok(Self::NasApi),
            "observe" | "tap" | "observe-tap" => Ok(Self::Observe),
            "ovs" => Ok(Self::Ovs),
            other => Err(format!("unknown e2b singleton component: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct E2bSingletonOutcome {
    pub sandbox_id: Option<String>,
    pub base_url: Option<String>,
    pub traffic_reachable: bool,
    pub message: Option<String>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0)
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

fn nas_api_port() -> u16 {
    std::env::var("CLAW_E2B_NAS_API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8090)
}

fn observe_live_port() -> u16 {
    std::env::var("CLAW_E2B_OBSERVE_LIVE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CLAW_TAP_LIVE_PORT)
}

async fn http_get_ok(url: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
    else {
        return false;
    };
    match client.get(url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

async fn wait_http_ok(url: &str, label: &str, max_attempts: u32) -> bool {
    for i in 1..=max_attempts {
        if http_get_ok(url).await {
            info!(
                target: "claw_e2b_singleton",
                url = %url,
                attempt = i,
                "{label} ready"
            );
            return true;
        }
        if i < max_attempts {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    false
}

async fn nas_api_health_ok(base_url: &str) -> bool {
    let url = format!("{}/healthz", base_url.trim_end_matches('/'));
    let Ok(client) = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
    else {
        return false;
    };
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("ok").and_then(serde_json::Value::as_bool))
            .unwrap_or(true),
        _ => false,
    }
}

async fn resolve_sandbox_id(
    client: &E2bSandboxClient,
    cluster_id: &str,
    claw_role: &str,
    pg_sandbox_id: Option<&str>,
) -> Option<String> {
    if let Some(pg_id) = pg_sandbox_id.map(str::trim).filter(|s| !s.is_empty()) {
        if client.sandbox_running(pg_id).await {
            return Some(pg_id.to_string());
        }
    }
    client
        .find_singleton(cluster_id, claw_role)
        .await
        .ok()
        .flatten()
}

async fn persist_nas_api(
    db: &GatewaySessionDb,
    base_url: &str,
    sandbox_id: &str,
) -> Result<(), sqlx::Error> {
    let (mut settings, tokens, _) = get_gateway_global_settings(db).await?;
    let now = now_ms();
    settings.e2b_nas_api = E2bNasApiSettings {
        template_id: settings.e2b_nas_api.template_id.clone(),
        base_url: Some(base_url.trim_end_matches('/').to_string()),
        sandbox_id: Some(sandbox_id.to_string()),
        updated_at_ms: now,
    };
    save_gateway_global_settings(db, &settings, &tokens, now).await
}

async fn persist_ovs(
    db: &GatewaySessionDb,
    base_url: &str,
    sandbox_id: &str,
) -> Result<(), sqlx::Error> {
    let (mut settings, tokens, _) = get_gateway_global_settings(db).await?;
    let now = now_ms();
    settings.e2b_ovs = E2bOvsSettings {
        template_id: settings.e2b_ovs.template_id.clone(),
        base_url: Some(base_url.trim_end_matches('/').to_string()),
        sandbox_id: Some(sandbox_id.to_string()),
        updated_at_ms: now,
    };
    save_gateway_global_settings(db, &settings, &tokens, now).await
}

async fn persist_observe_tap(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
    live_port: u16,
    live_base_url: &str,
) -> Result<(), sqlx::Error> {
    let (mut settings, tokens, _) = get_gateway_global_settings(db).await?;
    let now = now_ms();
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
    settings.claw_tap = ClawTapSettings {
        mode: ClawTapMode::Remote,
        host: proxy_host,
        proxy_port: DEFAULT_CLAW_TAP_PROXY_PORT,
        live_port,
        updated_at_ms: now,
        live_base_url: Some(live_base_url.to_string()),
        live_session_url_template: Some(live_session_viewer_url_template(live_base_url)),
        proxy_base_url: Some(proxy_base),
        e2b_observe_sandbox_id: Some(handle.sandbox_id.clone()),
    };
    save_gateway_global_settings(db, &settings, &tokens, now).await
}

async fn kill_existing_singleton(
    client: &E2bSandboxClient,
    cluster_id: &str,
    claw_role: &str,
    pg_sandbox_id: Option<&str>,
) {
    if let Some(sid) = resolve_sandbox_id(client, cluster_id, claw_role, pg_sandbox_id).await {
        info!(
            target: "claw_e2b_singleton",
            component = %claw_role,
            sandbox_id = %sid,
            "kill existing singleton before recreate"
        );
        let _ = client.kill_sandbox(&sid).await;
    }
}

async fn ensure_nas_api(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
) -> Result<E2bSingletonOutcome, String> {
    if !E2bNasApiSingleton::enabled_from_env() {
        return Ok(E2bSingletonOutcome {
            sandbox_id: None,
            base_url: None,
            traffic_reachable: true,
            message: Some("nas-api disabled".into()),
        });
    }
    let cluster_id = gateway_cluster_id()?;
    let port = nas_api_port();
    let template = load_e2b_nas_api_template_id(db)
        .await
        .map_err(|e| format!("load nas-api template: {e}"))?;
    let pg_sid = get_gateway_global_settings(db)
        .await
        .ok()
        .and_then(|(s, _, _)| s.e2b_nas_api.sandbox_id);

    let candidate = resolve_sandbox_id(
        client,
        &cluster_id,
        SINGLETON_ROLE_NAS_API,
        pg_sid.as_deref(),
    )
    .await;

    if let Some(ref sid) = candidate {
        let domain = client.config().domain.clone();
        let base_url = service_base_url(client, port, sid, &domain);
        if client.sandbox_running(sid).await && nas_api_health_ok(&base_url).await {
            client.touch_persistent_sandbox(sid).await?;
            let _ = persist_nas_api(db, &base_url, sid).await;
            let _ = client
                .reap_singleton_orphans(&cluster_id, SINGLETON_ROLE_NAS_API, sid)
                .await;
            info!(target: "claw_e2b_singleton", sandbox_id = %sid, "nas-api singleton online");
            return Ok(E2bSingletonOutcome {
                sandbox_id: Some(sid.clone()),
                base_url: Some(base_url),
                traffic_reachable: true,
                message: None,
            });
        }
        warn!(
            target: "claw_e2b_singleton",
            sandbox_id = %sid,
            "nas-api singleton unhealthy — recreate"
        );
        let _ = client.kill_sandbox(sid).await;
    }

    info!(
        target: "claw_e2b_singleton",
        template = %template,
        cluster_id = %cluster_id,
        "create nas-api singleton"
    );
    let handle = client
        .create_nas_api_singleton(&template, &cluster_id)
        .await?;
    let base_url = service_base_url(client, port, &handle.sandbox_id, &handle.sandbox_domain);
    let health_url = format!("{}/healthz", base_url.trim_end_matches('/'));
    let traffic_reachable = wait_http_ok(&health_url, "nas-api healthz", 60).await;
    if !traffic_reachable {
        let _ = client.kill_sandbox(&handle.sandbox_id).await;
        return Err(format!("nas-api healthz not reachable at {health_url}"));
    }
    persist_nas_api(db, &base_url, &handle.sandbox_id)
        .await
        .map_err(|e| format!("persist e2bNasApi: {e}"))?;
    let _ = client
        .reap_singleton_orphans(&cluster_id, SINGLETON_ROLE_NAS_API, &handle.sandbox_id)
        .await;
    Ok(E2bSingletonOutcome {
        sandbox_id: Some(handle.sandbox_id),
        base_url: Some(base_url),
        traffic_reachable: true,
        message: None,
    })
}

async fn ensure_observe(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
) -> Result<E2bSingletonOutcome, String> {
    if !e2b_observe_is_enabled() {
        return Ok(E2bSingletonOutcome {
            sandbox_id: None,
            base_url: None,
            traffic_reachable: true,
            message: Some("observe disabled".into()),
        });
    }
    let cluster_id = gateway_cluster_id()?;
    let template = load_e2b_observe_template_id(db)
        .await
        .map_err(|e| format!("load observe template: {e}"))?;
    let sandbox_db_url = sandbox_database_url()?;
    let live_port = observe_live_port();
    let pg_sid = get_gateway_global_settings(db)
        .await
        .ok()
        .and_then(|(s, _, _)| s.claw_tap.e2b_observe_sandbox_id.clone());

    let candidate = resolve_sandbox_id(
        client,
        &cluster_id,
        SINGLETON_ROLE_OBSERVE,
        pg_sid.as_deref(),
    )
    .await;

    if let Some(ref sid) = candidate {
        let domain = client.config().domain.clone();
        let live_base = service_base_url(client, live_port, sid, &domain);
        let proxy_base = service_base_url(client, DEFAULT_CLAW_TAP_PROXY_PORT, sid, &domain);
        let proxy_health = format!("{}/healthz", proxy_base.trim_end_matches('/'));
        let live_ok = http_get_ok(&live_base).await;
        let proxy_ok = fetch_tap_cluster_identity(&proxy_base, &cluster_id)
            .await
            .is_ok();
        if client.sandbox_running(sid).await
            && (live_ok || proxy_ok || http_get_ok(&proxy_health).await)
        {
            client.touch_persistent_sandbox(sid).await?;
            let handle = E2bSandboxHandle {
                sandbox_id: sid.clone(),
                sandbox_domain: domain,
                envd_access_token: None,
                traffic_access_token: None,
                ttyd_public_host: String::new(),
                ttyd_use_tls: !client.config().is_self_hosted(),
            };
            let _ = persist_observe_tap(db, client, &handle, live_port, &live_base).await;
            let _ = client
                .reap_singleton_orphans(&cluster_id, SINGLETON_ROLE_OBSERVE, sid)
                .await;
            info!(target: "claw_e2b_singleton", sandbox_id = %sid, "observe singleton online");
            return Ok(E2bSingletonOutcome {
                sandbox_id: Some(sid.clone()),
                base_url: Some(live_base),
                traffic_reachable: true,
                message: None,
            });
        }
        warn!(
            target: "claw_e2b_singleton",
            sandbox_id = %sid,
            "observe singleton unhealthy — recreate"
        );
        let _ = client.kill_sandbox(sid).await;
    }

    info!(
        target: "claw_e2b_singleton",
        template = %template,
        cluster_id = %cluster_id,
        "create observe singleton"
    );
    let handle = client
        .create_observe_singleton(&template, &cluster_id, &sandbox_db_url)
        .await?;
    let live_base = service_base_url(
        client,
        live_port,
        &handle.sandbox_id,
        &handle.sandbox_domain,
    );
    let traffic_reachable = wait_http_ok(&live_base, "observe Live", 60).await;
    if !traffic_reachable {
        let _ = client.kill_sandbox(&handle.sandbox_id).await;
        return Err(format!("observe Live not reachable at {live_base}"));
    }
    persist_observe_tap(db, client, &handle, live_port, &live_base)
        .await
        .map_err(|e| format!("persist clawTap: {e}"))?;
    let _ = client
        .reap_singleton_orphans(&cluster_id, SINGLETON_ROLE_OBSERVE, &handle.sandbox_id)
        .await;
    client.track_persistent_sandbox(&handle.sandbox_id);
    Ok(E2bSingletonOutcome {
        sandbox_id: Some(handle.sandbox_id),
        base_url: Some(live_base),
        traffic_reachable,
        message: None,
    })
}

async fn ensure_ovs(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
) -> Result<E2bSingletonOutcome, String> {
    let cluster_id = gateway_cluster_id()?;
    let template = load_e2b_ovs_template_id(db)
        .await
        .map_err(|e| format!("load ovs template: {e}"))?;
    let ovs_port = client.config().ovs_port;
    let pg_sid = get_gateway_global_settings(db)
        .await
        .ok()
        .and_then(|(s, _, _)| s.e2b_ovs.sandbox_id);

    let candidate =
        resolve_sandbox_id(client, &cluster_id, SINGLETON_ROLE_OVS, pg_sid.as_deref()).await;

    if let Some(ref sid) = candidate {
        let domain = client.config().domain.clone();
        let ovs_url = format!(
            "{}/ovs",
            service_base_url(client, ovs_port, sid, &domain).trim_end_matches('/')
        );
        let check_url = if ovs_url.ends_with('/') {
            ovs_url.clone()
        } else {
            format!("{ovs_url}/")
        };
        if client.sandbox_running(sid).await && http_get_ok(&check_url).await {
            client.touch_persistent_sandbox(sid).await?;
            let _ = persist_ovs(db, &ovs_url, sid).await;
            let _ = client
                .reap_singleton_orphans(&cluster_id, SINGLETON_ROLE_OVS, sid)
                .await;
            info!(target: "claw_e2b_singleton", sandbox_id = %sid, "ovs singleton online");
            return Ok(E2bSingletonOutcome {
                sandbox_id: Some(sid.clone()),
                base_url: Some(ovs_url),
                traffic_reachable: true,
                message: None,
            });
        }
        warn!(
            target: "claw_e2b_singleton",
            sandbox_id = %sid,
            "ovs singleton unhealthy — recreate"
        );
        let _ = client.kill_sandbox(sid).await;
    }

    info!(
        target: "claw_e2b_singleton",
        template = %template,
        cluster_id = %cluster_id,
        "create ovs singleton"
    );
    let handle = client.create_ovs_singleton(&template, &cluster_id).await?;
    let ovs_url = format!(
        "{}/ovs",
        service_base_url(client, ovs_port, &handle.sandbox_id, &handle.sandbox_domain,)
            .trim_end_matches('/')
    );
    let check_url = format!("{ovs_url}/");
    let traffic_reachable = wait_http_ok(&check_url, "OVS traffic", 60).await;
    if !traffic_reachable {
        let _ = client.kill_sandbox(&handle.sandbox_id).await;
        return Err(format!("OVS traffic not reachable at {check_url}"));
    }
    persist_ovs(db, &ovs_url, &handle.sandbox_id)
        .await
        .map_err(|e| format!("persist e2bOvs: {e}"))?;
    let _ = client
        .reap_singleton_orphans(&cluster_id, SINGLETON_ROLE_OVS, &handle.sandbox_id)
        .await;
    Ok(E2bSingletonOutcome {
        sandbox_id: Some(handle.sandbox_id),
        base_url: Some(ovs_url),
        traffic_reachable,
        message: None,
    })
}

pub async fn ensure_e2b_singleton(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    component: E2bSingletonComponent,
) -> Result<E2bSingletonOutcome, String> {
    match component {
        E2bSingletonComponent::NasApi => ensure_nas_api(db, client).await,
        E2bSingletonComponent::Observe => ensure_observe(db, client).await,
        E2bSingletonComponent::Ovs => ensure_ovs(db, client).await,
    }
}

pub async fn reset_e2b_singleton(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    component: E2bSingletonComponent,
) -> Result<E2bSingletonOutcome, String> {
    let cluster_id = gateway_cluster_id()?;
    match component {
        E2bSingletonComponent::NasApi => {
            let pg_sid = get_gateway_global_settings(db)
                .await
                .ok()
                .and_then(|(s, _, _)| s.e2b_nas_api.sandbox_id);
            kill_existing_singleton(
                client,
                &cluster_id,
                SINGLETON_ROLE_NAS_API,
                pg_sid.as_deref(),
            )
            .await;
            ensure_nas_api(db, client).await
        }
        E2bSingletonComponent::Observe => {
            let pg_sid = get_gateway_global_settings(db)
                .await
                .ok()
                .and_then(|(s, _, _)| s.claw_tap.e2b_observe_sandbox_id.clone());
            kill_existing_singleton(
                client,
                &cluster_id,
                SINGLETON_ROLE_OBSERVE,
                pg_sid.as_deref(),
            )
            .await;
            ensure_observe(db, client).await
        }
        E2bSingletonComponent::Ovs => {
            let pg_sid = get_gateway_global_settings(db)
                .await
                .ok()
                .and_then(|(s, _, _)| s.e2b_ovs.sandbox_id);
            kill_existing_singleton(client, &cluster_id, SINGLETON_ROLE_OVS, pg_sid.as_deref())
                .await;
            ensure_ovs(db, client).await
        }
    }
}

/// Startup: ensure observe / nas-api / ovs singletons exist, healthy, tracked for lease ticker.
pub async fn ensure_e2b_singletons_on_startup(db: &GatewaySessionDb, client: &E2bSandboxClient) {
    if !interactive_backend_is_e2b() {
        return;
    }
    if let Err(e) = ensure_nas_api(db, client).await {
        warn!(
            target: "claw_e2b_singleton",
            component = "nas-api",
            error = %e,
            "singleton ensure failed (best-effort)"
        );
    }
    if let Err(e) = ensure_observe(db, client).await {
        warn!(
            target: "claw_e2b_singleton",
            component = "observe",
            error = %e,
            "singleton ensure failed (best-effort)"
        );
    }
    if let Err(e) = ensure_ovs(db, client).await {
        warn!(
            target: "claw_e2b_singleton",
            component = "ovs",
            error = %e,
            "singleton ensure failed (best-effort)"
        );
    }
}

/// Periodic health reconcile — recreate unhealthy singletons (TTL handled by lease ticker).
pub fn spawn_singleton_health_reconcile_loop(
    db: Arc<GatewaySessionDb>,
    client: Arc<E2bSandboxClient>,
) {
    let interval_secs = e2b_project_worker_renew_interval_secs_from_env(3600);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            ensure_e2b_singletons_on_startup(db.as_ref(), client.as_ref()).await;
        }
    });
}
