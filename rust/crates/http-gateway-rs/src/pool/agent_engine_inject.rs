//! Install the project's selected Agent engine into a new e2b Worker. Author: kejiqing

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use claw_e2b_sandbox_client::{E2bSandboxClient, E2bSandboxHandle};
use tracing::info;

use crate::gateway_agent_engines::{load_agent_engines, AgentEngineEntry, AgentEngines};
use crate::session_db::GatewaySessionDb;

pub async fn run_agent_engine_inject(
    db: &GatewaySessionDb,
    client: &E2bSandboxClient,
    handle: &E2bSandboxHandle,
    harness_engine: &str,
) -> Result<(), String> {
    let requested = harness_engine.trim().to_ascii_lowercase();
    if requested.is_empty() || requested == "claw" {
        return Ok(());
    }
    let config = load_agent_engines(db).await.map_err(|e| e.to_string())?;
    let (engine_id, engine) = selected_engine(&config, &requested)?
        .ok_or_else(|| "native claw engine unexpectedly required an artifact".to_string())?;
    let script = guest_install_script()?;
    let env = BTreeMap::from([
        ("ENGINE_REF".to_string(), engine.r#ref.clone()),
        ("ENGINE_DIGEST".to_string(), engine.digest.clone()),
    ]);
    client
        .exec_shell_script_stdout_with(handle, &script, Some(&env), Some("root"), Some(900))
        .await
        .map_err(|e| format!("install Agent engine {engine_id:?}: {e}"))?;
    info!(
        target: "claw_agent_engine_inject",
        sandbox_id = %handle.sandbox_id,
        engine_id,
        engine_ref = %engine.r#ref,
        "Agent engine installed"
    );
    Ok(())
}

fn selected_engine<'a>(
    config: &'a AgentEngines,
    harness_engine: &str,
) -> Result<Option<(String, &'a AgentEngineEntry)>, String> {
    let engine_id = harness_engine.trim().to_ascii_lowercase();
    if engine_id.is_empty() || engine_id == "claw" {
        return Ok(None);
    }
    let engine = config
        .engines
        .get(&engine_id)
        .ok_or_else(|| format!("Agent engine {engine_id:?} is not configured"))?;
    Ok(Some((engine_id, engine)))
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

fn guest_install_script() -> Result<String, String> {
    let install_sh = deploy_e2b_file("guest-install-agent-engine.sh")?;
    Ok(format!(
        "set -euo pipefail\nbash <<'CLAW_AGENT_ENGINE_EOF'\n{install_sh}\nCLAW_AGENT_ENGINE_EOF\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn guest_script_downloads_one_tar_and_verifies_digest() {
        let _guard = crate::pool::config::test_env_lock();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let root = root.canonicalize().unwrap();
        let previous = std::env::var_os("CLAW_REPO_ROOT");
        std::env::set_var("CLAW_REPO_ROOT", &root);
        let script = guest_install_script().unwrap();
        match previous {
            Some(value) => std::env::set_var("CLAW_REPO_ROOT", value),
            None => std::env::remove_var("CLAW_REPO_ROOT"),
        }
        assert!(script.contains("ENGINE_REF is required"));
        assert!(script.contains("sha256sum -c"));
        assert!(script.contains("tar -xzf"));
    }

    #[test]
    fn selects_a_new_dynamic_engine_without_a_code_branch() {
        let entry = AgentEngineEntry {
            r#ref: "https://raw.example/future-v1.tar.gz".into(),
            digest: format!("sha256:{}", "a".repeat(64)),
        };
        let config = AgentEngines {
            engines: BTreeMap::from([("future-engine".into(), entry.clone())]),
        };
        let (id, selected) = selected_engine(&config, " Future-Engine ")
            .unwrap()
            .unwrap();
        assert_eq!(id, "future-engine");
        assert_eq!(selected, &entry);
        assert!(selected_engine(&config, "claw").unwrap().is_none());
    }
}
