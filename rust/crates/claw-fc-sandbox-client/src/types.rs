//! FC sandbox handle types. Author: kejiqing

use serde::Deserialize;

/// Created sandbox identity + ttyd reachability.
#[derive(Debug, Clone)]
pub struct FcSandboxHandle {
    pub sandbox_id: String,
    pub sandbox_domain: String,
    pub envd_access_token: Option<String>,
    pub traffic_access_token: Option<String>,
    /// Host for gateway → ttyd WebSocket (`{port}-{sandboxId}.{domain}`).
    pub ttyd_public_host: String,
    pub ttyd_use_tls: bool,
}

/// Result of `claw gateway-solve-once` inside FC sandbox.
#[derive(Debug, Clone)]
pub struct FcExecOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateSandboxResponse {
    #[serde(alias = "sandboxID")]
    pub sandbox_id: String,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default, alias = "envdAccessToken")]
    pub envd_access_token: Option<String>,
    #[serde(default, alias = "trafficAccessToken")]
    pub traffic_access_token: Option<String>,
}
