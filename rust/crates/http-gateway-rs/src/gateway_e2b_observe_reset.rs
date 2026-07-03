//! e2b observe tap reset — HTTP lifecycle in gateway-rs (no Python subprocess). Author: kejiqing

use claw_e2b_sandbox_client::E2bSandboxClient;
use serde::Serialize;
use tracing::{info, warn};

use crate::cluster_identity::{gateway_cluster_id, sandbox_database_url};
use crate::gateway_claw_tap_settings::{
    live_session_viewer_url_template, load_claw_tap_public, ClawTapMode, ClawTapSettings,
    ClawTapSettingsPublic, DEFAULT_CLAW_TAP_LIVE_PORT, DEFAULT_CLAW_TAP_PROXY_PORT,
};
use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::pool::interactive_backend::e2b_observe_is_enabled;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Serialize)]
pub struct ObserveTapResetResponse {
    pub tap: ClawTapSettingsPublic,
    #[serde(rename = "sandboxId")]
    pub sandbox_id: String,
    #[serde(rename = "liveBaseUrl")]
    pub live_base_url: String,
    #[serde(rename = "trafficReachable")]
    pub traffic_reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0)
}

fn observe_live_port() -> u16 {
    std::env::var("CLAW_E2B_OBSERVE_LIVE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_CLAW_TAP_LIVE_PORT)
}

fn internal_live_base(
    client: &E2bSandboxClient,
    sandbox_id: &str,
    domain: &str,
    live_port: u16,
) -> String {
    let host = client.service_public_host(live_port, sandbox_id, domain);
    let scheme = if client.config().is_self_hosted() {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{host}")
}

fn proxy_base_url(client: &E2bSandboxClient, sandbox_id: &str, domain: &str) -> String {
    let host = client.service_public_host(DEFAULT_CLAW_TAP_PROXY_PORT, sandbox_id, domain);
    let scheme = if client.config().is_self_hosted() {
        "http"
    } else {
        "https"
    };
    format!("{scheme}://{host}")
}

async fn observe_template_id(db: &GatewaySessionDb) -> String {
    if let Ok((settings_v, _, _)) = db.get_gateway_global_settings_raw().await {
        if let Some(tid) = settings_v
            .get("e2bObserve")
            .and_then(|o| o.get("templateId"))
            .and_then(|t| t.as_str())
        {
            let t = tid.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    std::env::var("CLAW_E2B_OBSERVE_TEMPLATE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "claw-observe".to_string())
}

async fn verify_live_traffic(live_base_url: &str) -> bool {
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(target: "gateway_observe_reset", error = %e, "live traffic http client");
            return false;
        }
    };
    match client.get(live_base_url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(e) => {
            warn!(
                target: "gateway_observe_reset",
                url = %live_base_url,
                error = %e,
                "live traffic probe failed"
            );
            false
        }
    }
}

async fn wait_live_traffic(live_base_url: &str) -> bool {
    const MAX_ATTEMPTS: u32 = 60;
    const SLEEP_SECS: u64 = 2;
    for i in 1..=MAX_ATTEMPTS {
        if verify_live_traffic(live_base_url).await {
            info!(
                target: "gateway_observe_reset",
                url = %live_base_url,
                attempt = i,
                "observe Live traffic ready"
            );
            return true;
        }
        if i < MAX_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_secs(SLEEP_SECS)).await;
        }
    }
    false
}

async fn persist_observe_tap(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    sandbox_id: &str,
    domain: &str,
    live_port: u16,
    live_base_url: &str,
) -> Result<(), sqlx::Error> {
    let (mut settings, tokens, _) = get_gateway_global_settings(db).await?;
    let now = now_ms();
    let proxy_host = client.service_public_host(DEFAULT_CLAW_TAP_PROXY_PORT, sandbox_id, domain);
    settings.claw_tap = ClawTapSettings {
        mode: ClawTapMode::Remote,
        host: proxy_host,
        proxy_port: DEFAULT_CLAW_TAP_PROXY_PORT,
        live_port,
        updated_at_ms: now,
        live_base_url: Some(live_base_url.to_string()),
        live_session_url_template: Some(live_session_viewer_url_template(live_base_url)),
        proxy_base_url: Some(proxy_base_url(client, sandbox_id, domain)),
        e2b_observe_sandbox_id: Some(sandbox_id.to_string()),
    };
    save_gateway_global_settings(db, &settings, &tokens, now).await
}

pub async fn reset_observe_tap(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
) -> Result<ObserveTapResetResponse, String> {
    if !e2b_observe_is_enabled() {
        return Err(
            "e2b observe tap disabled (CLAW_INTERACTIVE_BACKEND≠e2b or CLAW_E2B_OBSERVE=0)".into(),
        );
    }

    let cluster_id = gateway_cluster_id()?;
    let template = observe_template_id(db).await;
    let sandbox_db_url = sandbox_database_url()?;
    let live_port = observe_live_port();

    if let Some(existing) = client.find_observe_singleton(&cluster_id).await? {
        info!(
            target: "gateway_observe_reset",
            sandbox_id = %existing,
            "kill existing observe-singleton"
        );
        client.kill_sandbox(&existing).await?;
    }

    info!(
        target: "gateway_observe_reset",
        template = %template,
        cluster_id = %cluster_id,
        "create observe-singleton sandbox"
    );
    let handle = client
        .create_observe_singleton(&template, &cluster_id, &sandbox_db_url)
        .await?;
    let live_base_url = internal_live_base(
        client,
        &handle.sandbox_id,
        &handle.sandbox_domain,
        live_port,
    );

    info!(
        target: "gateway_observe_reset",
        sandbox_id = %handle.sandbox_id,
        url = %live_base_url,
        "wait for observe Live traffic"
    );
    let traffic_reachable = wait_live_traffic(&live_base_url).await;
    if !traffic_reachable {
        return Err(format!(
            "observe Live traffic not reachable at {live_base_url} — \
             rebuild claw-observe template or retry reset"
        ));
    }

    persist_observe_tap(
        db,
        client,
        &handle.sandbox_id,
        &handle.sandbox_domain,
        live_port,
        &live_base_url,
    )
    .await
    .map_err(|e| format!("persist clawTap to PG: {e}"))?;

    let tap = load_claw_tap_public(db).await.map_err(|e| e.to_string())?;
    Ok(ObserveTapResetResponse {
        tap,
        sandbox_id: handle.sandbox_id,
        live_base_url,
        traffic_reachable,
        message: None,
    })
}
