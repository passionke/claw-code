//! Platform worker.init: guest shell pulls CLI pins from registry (no host→envd binary push).
//! Same style as project `worker.init.*`: one root `run_sh`, sandbox does the work.
//! Author: kejiqing

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
    let script = guest_registry_install_script(image, expected_paths)?;
    let env = registry_pull_env(&image)?;
    client
        .exec_shell_script_stdout_with(
            handle,
            &script,
            Some(&env),
            Some("root"),
            Some(900),
        )
        .await
        .map_err(|e| format!("guest registry install {image}: {e}"))?;
    Ok(())
}

fn image_name_without_tag(image_ref: &str) -> &str {
    let file = image_ref.rsplit('/').next().unwrap_or(image_ref);
    if let Some(i) = file.rfind(':') {
        let abs = image_ref.len() - file.len() + i;
        &image_ref[..abs]
    } else {
        image_ref
    }
}

fn registry_host_of(image_ref: &str) -> Option<String> {
    let name = image_ref.split('@').next().unwrap_or(image_ref);
    let name = image_name_without_tag(name);
    let host = name.split('/').next()?.trim();
    if host.is_empty() || (!host.contains('.') && !host.contains(':') && host != "localhost") {
        return None;
    }
    Some(host.to_string())
}

fn registry_pull_env(image_ref: &str) -> Result<BTreeMap<String, String>, String> {
    let mut env = BTreeMap::new();
    if let Some(host) = registry_host_of(image_ref) {
        // Nexus group/pull ports are plain HTTP (see registry_extract.registry_scheme).
        let non_tls_port = host.rsplit_once(':').and_then(|(_, port)| {
            if port.chars().all(|c| c.is_ascii_digit()) && port != "443" && port != "8443" {
                Some(port)
            } else {
                None
            }
        });
        if non_tls_port.is_some() {
            env.insert("CLAW_REGISTRY_HTTP_HOSTS".into(), host);
        }
    }
    if let Some(cfg) = read_docker_config_json() {
        env.insert("CLAW_DOCKER_CONFIG_JSON".into(), cfg);
    }
    for (k, ek) in [
        ("ACR_USERNAME", "ACR_USERNAME"),
        ("ACR_USER", "ACR_USER"),
        ("ACR_PASSWORD", "ACR_PASSWORD"),
        ("ACR_PASSWORK", "ACR_PASSWORK"),
        ("CLAW_REGISTRY_USER", "CLAW_REGISTRY_USER"),
        ("CLAW_REGISTRY_PASSWORD", "CLAW_REGISTRY_PASSWORD"),
    ] {
        if let Ok(v) = std::env::var(k) {
            let t = v.trim();
            if !t.is_empty() {
                env.insert(ek.into(), t.to_string());
            }
        }
    }
    // Map CLAW_REGISTRY_* → ACR_* for registry_extract. Author: kejiqing
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
    Ok(env)
}

fn read_docker_config_json() -> Option<String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for key in ["CLAW_DOCKER_CONFIG", "DOCKER_CONFIG"] {
        if let Ok(raw) = std::env::var(key) {
            let p = PathBuf::from(raw.trim());
            if p.as_os_str().is_empty() {
                continue;
            }
            paths.push(if p.file_name().and_then(|s| s.to_str()) == Some("config.json") {
                p
            } else {
                p.join("config.json")
            });
        }
    }
    paths.extend([
        PathBuf::from("/run/claw/docker-config.json"),
        PathBuf::from("/run/claw/claw/docker-config.json"),
        dirs_next_home_docker_config(),
    ]);
    for p in paths {
        if let Ok(s) = std::fs::read_to_string(&p) {
            let t = s.trim();
            if t.starts_with('{') {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn dirs_next_home_docker_config() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/root"))
        .join(".docker/config.json")
}

fn registry_extract_py_source() -> Result<String, String> {
    let repo = std::env::var("CLAW_REPO_ROOT").unwrap_or_else(|_| {
        if Path::new("/app/deploy/e2b/registry_extract.py").exists() {
            "/app".into()
        } else {
            ".".into()
        }
    });
    let script = PathBuf::from(&repo).join("deploy/e2b/registry_extract.py");
    // Prefer mounted repo; fall back to image copy under /app.
    let path = if script.is_file() {
        script
    } else {
        PathBuf::from("/app/deploy/e2b/registry_extract.py")
    };
    std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "registry_extract.py not readable at {} (set CLAW_REPO_ROOT): {e}",
            path.display()
        )
    })
}

fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// worker.init-style guest script: ensure python3, pull OCI tree, install paths. Author: kejiqing
fn guest_registry_install_script(image_ref: &str, expected_paths: &[&str]) -> Result<String, String> {
    let extract_py = registry_extract_py_source()?;
    let image_q = shell_single_quote(image_ref);
    let mut checks = String::new();
    for p in expected_paths {
        let rel = p
            .trim_start_matches("/usr/local/")
            .trim_start_matches('/');
        let rel_q = shell_single_quote(rel);
        let dest_q = shell_single_quote(p);
        checks.push_str(&format!(
            r#"
src="$STAGE"/{rel_q}
dst={dest_q}
test -e "$src" || {{ echo "missing $src after registry extract" >&2; exit 1; }}
mkdir -p "$(dirname "$dst")"
if [ -d "$src" ]; then
  rm -rf "$dst"
  cp -a "$src" "$dst"
else
  install -m 0755 "$src" "$dst"
fi
"#
        ));
    }
    Ok(format!(
        r#"set -euo pipefail
# Platform CLI inject — guest pulls from registry (worker.init style). Author: kejiqing
if ! command -v python3 >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq python3 ca-certificates
fi
EXTRACT=/tmp/claw-registry-extract.py
cat >"$EXTRACT" <<'CLAW_REGISTRY_EXTRACT_EOF'
{extract_py}
CLAW_REGISTRY_EXTRACT_EOF
if [ -n "${{CLAW_DOCKER_CONFIG_JSON:-}}" ]; then
  mkdir -p /tmp/claw-docker
  printf '%s\n' "$CLAW_DOCKER_CONFIG_JSON" > /tmp/claw-docker/config.json
  export CLAW_DOCKER_CONFIG=/tmp/claw-docker/config.json
fi
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
python3 "$EXTRACT" --tree {image_q} /usr/local "$STAGE"
{checks}
"#
    ))
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
    fn image_name_keeps_registry_port() {
        assert_eq!(
            image_name_without_tag("registry.example:5000/ns/claw-cli/claw:tag"),
            "registry.example:5000/ns/claw-cli/claw"
        );
    }

    #[test]
    fn guest_script_is_worker_init_style_not_chunk_upload() {
        let _g = crate::pool::config::test_env_lock();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let root = root.canonicalize().unwrap();
        let prev = std::env::var_os("CLAW_REPO_ROOT");
        std::env::set_var("CLAW_REPO_ROOT", &root);
        let script = guest_registry_install_script(
            "repo.example/ns/claw-cli/claw:v1",
            &["/usr/local/bin/claw"],
        )
        .unwrap();
        match prev {
            Some(v) => std::env::set_var("CLAW_REPO_ROOT", v),
            None => std::env::remove_var("CLAW_REPO_ROOT"),
        }
        assert!(script.contains("CLAW_REGISTRY_EXTRACT_EOF"));
        assert!(script.contains("python3 \"$EXTRACT\" --tree"));
        assert!(!script.contains("upload chunk"));
        assert!(!script.contains("48_000"));
        assert!(!script.contains("base64 -d >>"));
    }
}
