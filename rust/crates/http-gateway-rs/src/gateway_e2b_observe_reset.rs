//! e2b observe tap reset — delegates to singleton lifecycle. Author: kejiqing

use claw_e2b_sandbox_client::E2bSandboxClient;

use crate::gateway_claw_tap_settings::{load_claw_tap_public, ClawTapSettingsPublic};
use crate::gateway_e2b_singleton_lifecycle::{reset_e2b_singleton, E2bSingletonComponent};
use crate::pool::interactive_backend::e2b_observe_is_enabled;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, serde::Serialize)]
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

pub async fn reset_observe_tap(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
) -> Result<ObserveTapResetResponse, String> {
    if !e2b_observe_is_enabled() {
        return Err(
            "e2b observe tap disabled (CLAW_E2B_OBSERVE=0)".into(),
        );
    }

    let outcome = reset_e2b_singleton(db, client, E2bSingletonComponent::Observe).await?;
    let sandbox_id = outcome
        .sandbox_id
        .ok_or_else(|| "observe reset succeeded but sandbox_id missing".to_string())?;
    let live_base_url = outcome
        .base_url
        .ok_or_else(|| "observe reset succeeded but live base URL missing".to_string())?;

    let tap = load_claw_tap_public(db).await.map_err(|e| e.to_string())?;
    Ok(ObserveTapResetResponse {
        tap,
        sandbox_id,
        live_base_url,
        traffic_reachable: outcome.traffic_reachable,
        message: outcome.message,
    })
}
