//! Platform worker.init: install claw / neuro / ACP engines from registry pins.
//! Runs before project preflight worker.init.*. Author: kejiqing

use std::path::{Path, PathBuf};
use std::process::Stdio;

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle};
use tracing::info;

use crate::gateway_cli_pins::{load_cli_pins, CliPinEntry, CliPins};
use crate::session_db::GatewaySessionDb;

/// Install pinned CLIs into a freshly created sandbox. Author: kejiqing
pub async fn run_platform_cli_inject(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
    harness_engine: &str,
) -> Result<(), String> {
    let pins = load_cli_pins(db).await.map_err(|e| e.to_string())?;
    let claw = pins
        .claw
        .as_ref()
        .ok_or_else(|| "cliPins.claw is not set — Admin must apply a claw CLI pin".to_string())?;
    inject_pin(client, handle, claw, &["/usr/local/bin/claw"]).await?;

    let engine = harness_engine.trim().to_ascii_lowercase();
    match engine.as_str() {
        "" | "claw" => {}
        "opencode" => {
            inject_optional(
                client,
                handle,
                &pins.neuro_opencode,
                "cliPins.neuroOpencode",
                &["/usr/local/bin/neuro-opencode"],
            )
            .await?;
            inject_optional(
                client,
                handle,
                &pins.acp_opencode,
                "cliPins.acpOpencode",
                &["/usr/local/lib/neuro-engines/opencode/bin/opencode"],
            )
            .await?;
        }
        "appserver" => {
            inject_optional(
                client,
                handle,
                &pins.neuro_appserver,
                "cliPins.neuroAppserver",
                &["/usr/local/bin/neuro-appserver"],
            )
            .await?;
            inject_optional(
                client,
                handle,
                &pins.acp_appserver,
                "cliPins.acpAppserver",
                &["/usr/local/lib/neuro-engines/codex-acp"],
            )
            .await?;
        }
        other => {
            return Err(format!("unsupported harnessEngine for cli inject: {other}"));
        }
    }

    // Smoke: claw must exist after inject.
    let out = client
        .exec_shell_script_stdout_with(
            handle,
            "set -euo pipefail; command -v claw; claw --version | head -1",
            None,
            Some("root"),
            Some(120),
        )
        .await
        .map_err(|e| format!("platform cli inject smoke failed: {e}"))?;
    info!(
        target: "claw_platform_cli_inject",
        sandbox_id = %handle.sandbox_id,
        smoke = %out.trim(),
        "platform CLI inject ok"
    );
    Ok(())
}

async fn inject_optional(
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
    pin: &Option<CliPinEntry>,
    label: &str,
    expected_paths: &[&str],
) -> Result<(), String> {
    let Some(pin) = pin else {
        return Err(format!("{label} is not set"));
    };
    inject_pin(client, handle, pin, expected_paths).await
}

async fn inject_pin(
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
    pin: &CliPinEntry,
    expected_paths: &[&str],
) -> Result<(), String> {
    let staging = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
    extract_artifact_to(&pin.r#ref, staging.path()).await?;
    for rel in expected_paths {
        // --tree /usr/local → files land under staging/{bin,lib,...}
        let under_local = staging.path().join(
            rel.trim_start_matches("/usr/local/")
                .trim_start_matches('/'),
        );
        let host_path = if under_local.exists() {
            under_local
        } else {
            let abs = staging.path().join(rel.trim_start_matches('/'));
            if abs.exists() {
                abs
            } else {
                return Err(format!(
                    "extracted {} missing expected path {rel} (under {})",
                    pin.r#ref,
                    staging.path().display()
                ));
            }
        };
        upload_path_to_guest(client, handle, &host_path, rel).await?;
    }
    Ok(())
}

async fn extract_artifact_to(image_ref: &str, dest: &Path) -> Result<(), String> {
    let repo = std::env::var("CLAW_REPO_ROOT").unwrap_or_else(|_| {
        if Path::new("/app/deploy/e2b/registry_extract.py").exists() {
            "/app".into()
        } else {
            ".".into()
        }
    });
    let script = PathBuf::from(&repo).join("deploy/e2b/registry_extract.py");
    if !script.is_file() {
        return Err(format!(
            "registry_extract.py not found at {} (set CLAW_REPO_ROOT)",
            script.display()
        ));
    }
    let out = tokio::process::Command::new("python3")
        .arg(&script)
        .arg("--tree")
        .arg(image_ref)
        .arg("/usr/local")
        .arg(dest)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("registry extract spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "registry extract failed for {image_ref}: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}

async fn upload_path_to_guest(
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
    host_path: &Path,
    guest_path: &str,
) -> Result<(), String> {
    if host_path.is_dir() {
        // tar | base64 stream directory
        let tar = tokio::process::Command::new("tar")
            .args(["-C", host_path.to_str().unwrap_or("."), "-czf", "-", "."])
            .stdout(Stdio::piped())
            .output()
            .await
            .map_err(|e| format!("tar: {e}"))?;
        if !tar.status.success() {
            return Err("tar of cli artifact tree failed".into());
        }
        let b64 = base64_encode(&tar.stdout);
        let parent = guest_path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
        let script = format!(
            r"set -euo pipefail
mkdir -p {parent} {guest_path}
echo '{b64}' | base64 -d | tar -xzf - -C {guest_path}
"
        );
        client
            .exec_shell_script_stdout_with(handle, &script, None, Some("root"), Some(600))
            .await
            .map_err(|e| format!("upload tree {guest_path}: {e}"))?;
        return Ok(());
    }

    let bytes = tokio::fs::read(host_path)
        .await
        .map_err(|e| format!("read {}: {e}", host_path.display()))?;
    // Chunk large binaries (base64 ~1.3x).
    const CHUNK: usize = 48_000;
    let parent = guest_path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
    client
        .exec_shell_script_stdout_with(
            handle,
            &format!("mkdir -p {parent}; : > {guest_path}"),
            None,
            Some("root"),
            Some(60),
        )
        .await
        .map_err(|e| format!("prep {guest_path}: {e}"))?;
    for chunk in bytes.chunks(CHUNK) {
        let b64 = base64_encode(chunk);
        let script = format!(
            r"set -euo pipefail
echo '{b64}' | base64 -d >> {guest_path}
"
        );
        client
            .exec_shell_script_stdout_with(handle, &script, None, Some("root"), Some(120))
            .await
            .map_err(|e| format!("upload chunk {guest_path}: {e}"))?;
    }
    client
        .exec_shell_script_stdout_with(
            handle,
            &format!("chmod 0755 {guest_path}"),
            None,
            Some("root"),
            Some(30),
        )
        .await
        .map_err(|e| format!("chmod {guest_path}: {e}"))?;
    Ok(())
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Exposed for tests / diagnostics. Author: kejiqing
#[allow(dead_code)]
pub fn pins_summary(pins: &CliPins) -> String {
    format!(
        "claw={} neuro_opencode={} acp_opencode={}",
        pins.claw.as_ref().map(|p| p.r#ref.as_str()).unwrap_or("-"),
        pins.neuro_opencode
            .as_ref()
            .map(|p| p.r#ref.as_str())
            .unwrap_or("-"),
        pins.acp_opencode
            .as_ref()
            .map(|p| p.r#ref.as_str())
            .unwrap_or("-"),
    )
}
