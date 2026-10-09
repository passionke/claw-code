//! Platform worker.init: run `deploy/e2b/guest-install-cli-pin.sh` in the sandbox.
//! Pin ref is http(s) or file:// tar.gz. Author: kejiqing

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
                pins.neuro_opencode.as_ref(),
                "cliPins.neuroOpencode",
                &["/usr/local/bin/neuro-opencode"],
            )
            .await?;
            inject_optional(
                client,
                handle,
                pins.acp_opencode.as_ref(),
                "cliPins.acpOpencode",
                &["/usr/local/lib/neuro-engines/opencode/bin/opencode"],
            )
            .await?;
        }
        "appserver" => {
            inject_optional(
                client,
                handle,
                pins.neuro_appserver.as_ref(),
                "cliPins.neuroAppserver",
                &["/usr/local/bin/neuro-appserver"],
            )
            .await?;
            inject_optional(
                client,
                handle,
                pins.acp_appserver.as_ref(),
                "cliPins.acpAppserver",
                &["/usr/local/lib/neuro-engines/codex-acp"],
            )
            .await?;
        }
        other => {
            return Err(format!("unsupported harnessEngine for cli inject: {other}"));
        }
    }

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
    pin: Option<&CliPinEntry>,
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
    let image = pin.r#ref.trim();
    if image.is_empty() {
        return Err("cli pin ref must be non-empty".into());
    }
    let script = guest_run_sh()?;
    let mut env = registry_auth_env();
    env.insert("IMAGE_REF".into(), image.to_string());
    env.insert("EXPECTED_PATHS".into(), expected_paths.join(" "));
    client
        .exec_shell_script_stdout_with(handle, &script, Some(&env), Some("root"), Some(900))
        .await
        .map_err(|e| format!("guest-install-cli-pin {image}: {e}"))?;
    Ok(())
}

fn deploy_e2b_file(name: &str) -> Result<String, String> {
    let repo = std::env::var("CLAW_REPO_ROOT").unwrap_or_else(|_| {
        if Path::new("/app/deploy/e2b").is_dir() {
            "/app".into()
        } else {
            ".".into()
        }
    });
    let mounted = PathBuf::from(&repo).join("deploy/e2b").join(name);
    let bundled = PathBuf::from("/app/deploy/e2b").join(name);
    let path = if mounted.is_file() { mounted } else { bundled };
    std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{} not readable at {} (set CLAW_REPO_ROOT): {e}",
            name,
            path.display()
        )
    })
}

/// Run guest-install-cli-pin.sh in the sandbox (worker.init). Author: kejiqing
fn guest_run_sh() -> Result<String, String> {
    let install_sh = deploy_e2b_file("guest-install-cli-pin.sh")?;
    Ok(format!(
        "set -euo pipefail\nbash <<'CLAW_GUEST_INSTALL_EOF'\n{install_sh}\nCLAW_GUEST_INSTALL_EOF\n"
    ))
}

fn registry_auth_env() -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for k in [
        "ACR_USERNAME",
        "ACR_USER",
        "ACR_PASSWORD",
        "ACR_PASSWORK",
        "CLAW_REGISTRY_USER",
        "CLAW_REGISTRY_PASSWORD",
    ] {
        if let Ok(v) = std::env::var(k) {
            let t = v.trim();
            if !t.is_empty() {
                env.insert(k.into(), t.to_string());
            }
        }
    }
    if !env.contains_key("ACR_USERNAME") && !env.contains_key("ACR_USER") {
        if let Some(u) = env.get("CLAW_REGISTRY_USER").cloned() {
            env.insert("ACR_USERNAME".into(), u);
        }
    }
    if !env.contains_key("ACR_PASSWORD") && !env.contains_key("ACR_PASSWORK") {
        if let Some(p) = env.get("CLAW_REGISTRY_PASSWORD").cloned() {
            env.insert("ACR_PASSWORD".into(), p);
        }
    }
    env
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_run_sh_loads_repo_script_not_chunk_upload() {
        let _g = crate::pool::config::test_env_lock();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let root = root.canonicalize().unwrap();
        let prev = std::env::var_os("CLAW_REPO_ROOT");
        std::env::set_var("CLAW_REPO_ROOT", &root);
        let script = guest_run_sh().unwrap();
        match prev {
            Some(v) => std::env::set_var("CLAW_REPO_ROOT", v),
            None => std::env::remove_var("CLAW_REPO_ROOT"),
        }
        assert!(script.contains("IMAGE_REF is required"));
        assert!(script.contains("file://*"));
        assert!(script.contains("tar -xzf"));
        assert!(script.contains("curl -fsSL"));
        assert!(!script.contains("registry_extract"));
        assert!(!script.contains("48_000"));
    }
}
