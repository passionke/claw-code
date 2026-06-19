//! Local clawTap lifecycle (tap-down / tap-up) when stack scripts are reachable. Author: kejiqing

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Serialize;
use tokio::process::Command;

use crate::gateway_claw_tap_settings::{ClawTapMode, ClawTapSettings};
use crate::gateway_global_settings::get_gateway_global_settings;
use crate::session_db::GatewaySessionDb;

#[derive(Debug, Clone, Serialize)]
pub struct TapRestartOutcome {
    pub attempted: bool,
    pub restarted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[must_use]
pub fn is_local_claw_tap(settings: &ClawTapSettings) -> bool {
    settings.mode == ClawTapMode::Local
}

/// Host `deploy/stack` directory (absolute). Set by compose-include in `.claw-llm-runtime.env`.
#[must_use]
pub fn resolve_stack_dir() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("CLAW_STACK_DIR") {
        let p = PathBuf::from(raw.trim());
        if p.join("lib/tap-up.sh").is_file() {
            return Some(p);
        }
    }
    if let Ok(root) = std::env::var("CLAW_REPO_ROOT") {
        let repo = PathBuf::from(root.trim());
        let stack = repo.join("deploy/stack");
        if stack.join("lib/tap-up.sh").is_file() {
            return Some(stack);
        }
    }
    None
}

async fn run_bash(script: &Path, stack_dir: &Path, live_port: u16) -> Result<(), String> {
    let status = Command::new("bash")
        .arg(script)
        .env("CLAUDE_TAP_LIVE_PORT", live_port.to_string())
        .current_dir(stack_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .await
        .map_err(|e| format!("run {}: {e}", script.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with {}",
            script
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("script"),
            status.code().unwrap_or(-1)
        ))
    }
}

/// Best-effort local tap restart via `deploy/stack/lib/tap-down.sh` + `tap-up.sh`.
pub async fn restart_local_claw_tap(live_port: u16) -> TapRestartOutcome {
    let Some(stack_dir) = resolve_stack_dir() else {
        return TapRestartOutcome {
            attempted: false,
            restarted: false,
            message: Some(
                "tap restart skipped (CLAW_STACK_DIR not set; upstream hot-reload or gateway.sh tap-up)"
                    .into(),
            ),
        };
    };
    let down = stack_dir.join("lib/tap-down.sh");
    let up = stack_dir.join("lib/tap-up.sh");
    if !up.is_file() {
        return TapRestartOutcome {
            attempted: false,
            restarted: false,
            message: Some(format!("tap restart skipped (missing {})", up.display())),
        };
    }
    if down.is_file() {
        let _ = run_bash(&down, &stack_dir, live_port).await;
    }
    match run_bash(&up, &stack_dir, live_port).await {
        Ok(()) => TapRestartOutcome {
            attempted: true,
            restarted: true,
            message: Some("local clawTap restarted".into()),
        },
        Err(e) => TapRestartOutcome {
            attempted: true,
            restarted: false,
            message: Some(format!("tap-up failed: {e}")),
        },
    }
}

pub async fn restart_local_claw_tap_if_configured(db: &GatewaySessionDb) -> TapRestartOutcome {
    let settings = match get_gateway_global_settings(db).await {
        Ok((s, _, _)) => s.claw_tap,
        Err(e) => {
            return TapRestartOutcome {
                attempted: false,
                restarted: false,
                message: Some(format!("load clawTap settings: {e}")),
            };
        }
    };
    if !is_local_claw_tap(&settings) {
        return TapRestartOutcome {
            attempted: false,
            restarted: false,
            message: None,
        };
    }
    restart_local_claw_tap(settings.live_port).await
}
