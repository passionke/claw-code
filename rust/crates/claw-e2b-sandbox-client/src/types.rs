//! e2b sandbox handle types. Author: kejiqing

use serde::Deserialize;

/// Created sandbox identity + ttyd reachability.
#[derive(Debug, Clone)]
pub struct E2bSandboxHandle {
    pub sandbox_id: String,
    pub sandbox_domain: String,
    pub envd_access_token: Option<String>,
    pub traffic_access_token: Option<String>,
    /// Host for gateway → ttyd WebSocket (`{port}-{sandboxId}.{domain}`).
    pub ttyd_public_host: String,
    pub ttyd_use_tls: bool,
    /// Relaxed worker built-in OVS (`{ovs_port}-{sandboxId}.{domain}`).
    pub ovs_public_host: Option<String>,
    /// `http(s)://{ovs_public_host}/ovs`
    pub ovs_base_url: Option<String>,
}

/// Result of `claw gateway-solve-once` inside e2b sandbox.
#[derive(Debug, Clone)]
pub struct E2bExecOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Per-turn solve inputs delivered inline to the worker (no claw-nas-api write). Author: kejiqing
#[derive(Debug, Clone, Copy, Default)]
pub struct GatewaySolveInputs<'a> {
    /// Serialized `gateway-solve-task.json` (empty → worker reads the existing `task_file`).
    pub task_json: &'a str,
    /// Optional session transcript seeded under `.claw/gateway-solve-session.jsonl`.
    pub session_jsonl: Option<&'a str>,
    /// Session directory segment under `/claw_sessions/{segment}`.
    pub session_segment: &'a str,
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
