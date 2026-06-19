//! FC sandbox env config. Author: kejiqing

use std::path::PathBuf;

/// Resolved FC / E2B API settings from environment.
#[derive(Debug, Clone)]
pub struct FcSandboxConfig {
    pub api_key: String,
    pub api_url: String,
    /// Client proxy base (`http://10.8.0.9:3002` for self-hosted e2bserver).
    pub sandbox_url: Option<String>,
    pub domain: String,
    pub template: String,
    pub sandbox_timeout_secs: u64,
    pub nas_server: Option<String>,
    pub nas_export: Option<String>,
    pub nas_volume_name: Option<String>,
    pub nas_tools_rel: String,
    pub nas_user_id: u32,
    pub nas_group_id: u32,
    pub exec_helper: PathBuf,
    pub ttyd_port: u16,
}

impl FcSandboxConfig {
    /// Load from process environment (repo root `.env` after gateway sources it).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("CLAW_FC_API_KEY")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                std::env::var("ALIYUN_E2B_TOKEN")
                    .ok()
                    .filter(|v| !v.trim().is_empty())
            })?;
        let api_url = std::env::var("CLAW_FC_API_URL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "https://api.cn-beijing.e2b.fc.aliyuncs.com".into());
        let domain = std::env::var("CLAW_FC_DOMAIN")
            .ok()
            .or_else(|| std::env::var("E2B_DOMAIN").ok())
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "cn-beijing.e2b.fc.aliyuncs.com".into());
        let sandbox_url = std::env::var("CLAW_E2B_SANDBOX_URL")
            .ok()
            .or_else(|| std::env::var("E2B_SANDBOX_URL").ok())
            .filter(|v| !v.trim().is_empty());
        let template = std::env::var("CLAW_FC_TEMPLATE")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "code-interpreter-v1".into());
        let sandbox_timeout_secs = std::env::var("CLAW_FC_SANDBOX_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600);
        let nas_server = std::env::var("CLAW_FC_NAS_SERVER")
            .ok()
            .or_else(|| std::env::var("NAS_BASE_URL").ok())
            .filter(|v| !v.trim().is_empty());
        let nas_export = std::env::var("CLAW_FC_NAS_EXPORT")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let nas_volume_name = std::env::var("CLAW_FC_NAS_VOLUME_NAME")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let nas_tools_rel = std::env::var("CLAW_FC_NAS_TOOLS_REL")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| ".claw-fc-tools".into());
        let nas_user_id = std::env::var("CLAW_WORKER_UID")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000);
        let nas_group_id = std::env::var("CLAW_WORKER_GID")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1000);
        let exec_helper = std::env::var("CLAW_FC_EXEC_HELPER")
            .ok()
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(default_exec_helper_path);
        let ttyd_port = std::env::var("CLAW_FC_TTYD_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7681);
        Some(Self {
            api_key,
            api_url: api_url.trim_end_matches('/').to_string(),
            sandbox_url: sandbox_url.map(|u| u.trim_end_matches('/').to_string()),
            domain,
            template,
            sandbox_timeout_secs,
            nas_server,
            nas_export,
            nas_volume_name,
            nas_tools_rel,
            nas_user_id,
            nas_group_id,
            exec_helper,
            ttyd_port,
        })
    }

    /// Self-hosted e2bserver (`passionke/e2bserver`) vs Alibaba FC cloud sandbox.
    #[must_use]
    pub fn is_self_hosted(&self) -> bool {
        let u = self.api_url.to_ascii_lowercase();
        !(u.contains("aliyuncs.com") || u.contains("e2b.fc."))
    }
}

fn default_exec_helper_path() -> PathBuf {
    PathBuf::from("deploy/fc-sandbox/fc_exec.py")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_exec_helper_relative_to_repo() {
        let p = default_exec_helper_path();
        assert!(p.to_string_lossy().contains("fc_exec.py"));
    }
}
