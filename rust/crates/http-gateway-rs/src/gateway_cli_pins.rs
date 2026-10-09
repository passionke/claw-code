//! Global CLI version pins (full registry ref + digest). Author: kejiqing
//! Applied via Admin / PUT — never by CI writing production PG.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::gateway_global_settings::{get_gateway_global_settings, save_gateway_global_settings};
use crate::session_db::GatewaySessionDb;

/// One pinned CLI artifact. Author: kejiqing
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CliPinEntry {
    /// Full registry ref, e.g. `nora.example/passionke/claw-cli/claw:v1.8.20`.
    pub r#ref: String,
    /// Content digest `sha256:…` (empty until resolve succeeds).
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
    e.digest = e.digest.trim().to_string();
    Ok(e)
}

/// Resolve digest via registry manifest GET when missing. Author: kejiqing
async fn ensure_digest(entry: &mut CliPinEntry) -> Result<(), String> {
    if entry.digest.starts_with("sha256:") && entry.digest.len() > 20 {
        return Ok(());
    }
    let digest = probe_image_digest(&entry.r#ref).await?;
    if digest.is_empty() {
        return Err(format!(
            "registry probe failed for {} (image missing or unauthorized)",
            entry.r#ref
        ));
    }
    entry.digest = digest;
    Ok(())
}

async fn probe_image_digest(image_ref: &str) -> Result<String, String> {
    // Prefer skopeo when present (same as pack path).
    if let Ok(out) = tokio::process::Command::new("skopeo")
        .args(["inspect", "--format", "{{.Digest}}", &format!("docker://{image_ref}")])
        .output()
        .await
    {
        if out.status.success() {
            let d = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if d.starts_with("sha256:") {
                return Ok(d);
            }
        }
    }
    // Fallback: python registry_extract try_image_digest if repo is mounted.
    let script = std::env::var("CLAW_REPO_ROOT").unwrap_or_else(|_| "/app".into());
    let py = format!("{script}/deploy/e2b/registry_extract.py");
    if tokio::fs::try_exists(&py).await.unwrap_or(false) {
        let code = format!(
            "from registry_extract import try_image_digest; import sys; d=try_image_digest(sys.argv[1]) or ''; print(d)",
        );
        let out = tokio::process::Command::new("python3")
            .args(["-c", &code, image_ref])
            .current_dir(format!("{script}/deploy/e2b"))
            .output()
            .await
            .map_err(|e| format!("python digest probe: {e}"))?;
        if out.status.success() {
            let d = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if d.starts_with("sha256:") {
                return Ok(d);
            }
        }
    }
    Err(format!(
        "cannot resolve digest for {image_ref}; ensure image exists and skopeo/registry auth works"
    ))
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
}
