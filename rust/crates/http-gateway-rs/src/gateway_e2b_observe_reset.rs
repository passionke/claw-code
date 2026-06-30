//! e2b observe tap reset — delegate lifecycle to `e2b-tap-live-up.py`. Author: kejiqing

use std::path::PathBuf;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::gateway_claw_tap_settings::{load_claw_tap_public, ClawTapSettingsPublic};
use crate::pool::interactive_backend::e2b_observe_is_enabled;
use crate::pool_worker_runtime_sync;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Deserialize)]
struct FcTapLiveUpJson {
    #[serde(rename = "liveBaseUrl")]
    live_base_url: String,
    #[serde(rename = "sandboxId")]
    sandbox_id: String,
    #[serde(rename = "trafficReachable", default)]
    traffic_reachable: bool,
}

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

fn resolve_fc_tap_live_up_script() -> Result<PathBuf, String> {
    let repo = pool_worker_runtime_sync::resolve_repo_root()
        .ok_or_else(|| "CLAW_REPO_ROOT unset; cannot run e2b-tap-live-up.py".to_string())?;
    let script = repo.join("deploy/e2b/e2b-tap-live-up.py");
    if !script.is_file() {
        return Err(format!("missing {}", script.display()));
    }
    Ok(script)
}

async fn run_fc_tap_live_up_reset() -> Result<FcTapLiveUpJson, String> {
    let script = resolve_fc_tap_live_up_script()?;
    let repo = script
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .ok_or_else(|| "invalid e2b-tap-live-up.py path".to_string())?;
    let output = Command::new("python3")
        .arg(&script)
        .arg("--reset")
        .arg("--json")
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("e2b-tap-live-up --reset: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            stdout.trim()
        } else {
            stderr.trim()
        };
        return Err(format!(
            "e2b-tap-live-up --reset exit {}: {}",
            output.status.code().unwrap_or(-1),
            detail
        ));
    }
    serde_json::from_str(stdout.trim())
        .map_err(|e| format!("parse e2b-tap-live-up JSON: {e}: {stdout}"))
}

pub async fn reset_observe_tap(db: &GatewaySessionDb) -> Result<ObserveTapResetResponse, String> {
    if !e2b_observe_is_enabled() {
        return Err(
            "e2b observe tap disabled (CLAW_INTERACTIVE_BACKEND≠e2b or CLAW_E2B_OBSERVE=0)".into(),
        );
    }
    let urls = run_fc_tap_live_up_reset().await?;
    let tap = load_claw_tap_public(db).await.map_err(|e| e.to_string())?;
    Ok(ObserveTapResetResponse {
        tap,
        sandbox_id: urls.sandbox_id,
        live_base_url: urls.live_base_url,
        traffic_reachable: urls.traffic_reachable,
        message: if urls.traffic_reachable {
            None
        } else {
            Some("observe sandbox started but traffic check failed".into())
        },
    })
}
