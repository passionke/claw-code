//! Global CLI version pins (full registry ref + digest). Author: kejiqing
//! Applied via Admin / PUT — never by CI writing production PG.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::session_db::GatewaySessionDb;

/// One pinned CLI tar.gz. Author: kejiqing
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CliPinEntry {
    /// `http(s)://…/claw-….tar.gz` (Nexus raw) or `file:///abs/path.tar.gz`.
    pub r#ref: String,
    /// Content digest `sha256:…` of the tar.gz (optional).
    #[serde(default)]
    pub digest: String,
}

/// Platform CLI pins in `settings_json.cliPins`. Author: kejiqing
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CliPins {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claw: Option<CliPinEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neuro_opencode: Option<CliPinEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neuro_appserver: Option<CliPinEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_opencode: Option<CliPinEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_appserver: Option<CliPinEntry>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PutCliPinsInput {
    #[serde(default)]
    pub claw: Option<CliPinEntry>,
    #[serde(default)]
    pub neuro_opencode: Option<CliPinEntry>,
    #[serde(default)]
    pub neuro_appserver: Option<CliPinEntry>,
    #[serde(default)]
    pub acp_opencode: Option<CliPinEntry>,
    #[serde(default)]
    pub acp_appserver: Option<CliPinEntry>,
}

fn normalize_entry(mut e: CliPinEntry) -> Result<CliPinEntry, String> {
    e.r#ref = e.r#ref.trim().to_string();
    if e.r#ref.is_empty() {
        return Err("cli pin ref must be non-empty".into());
    }
    if e.r#ref.contains(' ') {
        return Err("cli pin ref must not contain spaces".into());
    }
    let ok = e.r#ref.starts_with("http://")
        || e.r#ref.starts_with("https://")
        || e.r#ref.starts_with("file://");
    if !ok {
        return Err("cli pin ref must be http(s):// or file:// tar.gz".into());
    }
    e.digest = e.digest.trim().to_string();
    Ok(e)
}

/// Fill digest from a local file:// tar.gz when the gateway can see it. Author: kejiqing
async fn ensure_digest(entry: &mut CliPinEntry) -> Result<(), String> {
    if entry.digest.starts_with("sha256:") && entry.digest.len() > 20 {
        return Ok(());
    }
    if let Some(path) = entry.r#ref.strip_prefix("file://") {
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        if let Ok(bytes) = tokio::fs::read(&path).await {
            use sha2::{Digest, Sha256};
            let h = Sha256::digest(&bytes);
            entry.digest = format!("sha256:{h:x}");
        }
    }
    Ok(())
}

pub async fn load_cli_pins(db: &GatewaySessionDb) -> Result<CliPins, sqlx::Error> {
    let (settings, _, _) = get_gateway_global_settings(db).await?;
    Ok(settings.cli_pins)
}

pub async fn put_cli_pins(
    db: &GatewaySessionDb,
    input: PutCliPinsInput,
) -> Result<CliPins, String> {
    let (mut settings, tokens, _) = get_gateway_global_settings(db)
        .await
        .map_err(|e| e.to_string())?;
    merge_pin(&mut settings.cli_pins.claw, input.claw).await?;
    merge_pin(&mut settings.cli_pins.neuro_opencode, input.neuro_opencode).await?;
    merge_pin(
        &mut settings.cli_pins.neuro_appserver,
        input.neuro_appserver,
    )
    .await?;
    merge_pin(&mut settings.cli_pins.acp_opencode, input.acp_opencode).await?;
    merge_pin(&mut settings.cli_pins.acp_appserver, input.acp_appserver).await?;
    let now = chrono::Utc::now().timestamp_millis();
    save_gateway_global_settings(db, &settings, &tokens, now)
        .await
        .map_err(|e| e.to_string())?;
    Ok(settings.cli_pins)
}

async fn merge_pin(
    slot: &mut Option<CliPinEntry>,
    incoming: Option<CliPinEntry>,
) -> Result<(), String> {
    let Some(raw) = incoming else {
        return Ok(());
    };
    let mut e = normalize_entry(raw)?;
    ensure_digest(&mut e).await?;
    *slot = Some(e);
    Ok(())
}

/// Merge into store when salvaging JSON. Author: kejiqing
pub fn cli_pins_from_value(v: &serde_json::Value) -> CliPins {
    serde_json::from_value(v.get("cliPins").cloned().unwrap_or(serde_json::json!({})))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rejects_empty() {
        assert!(normalize_entry(CliPinEntry {
            r#ref: "  ".into(),
            digest: String::new(),
        })
        .is_err());
    }

    #[test]
    fn normalize_accepts_http_and_file() {
        assert!(normalize_entry(CliPinEntry {
            r#ref: "http://nexus.example/repository/raw/claw-cli/claw-v1.tar.gz".into(),
            digest: String::new(),
        })
        .is_ok());
        assert!(normalize_entry(CliPinEntry {
            r#ref: "file:///opt/claw/claw.tar.gz".into(),
            digest: String::new(),
        })
        .is_ok());
        assert!(normalize_entry(CliPinEntry {
            r#ref: "repo.example/passionke/claw-cli/claw:v1".into(),
            digest: String::new(),
        })
        .is_err());
    }
}
