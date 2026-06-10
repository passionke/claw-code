//! Sandbox registry row shape (evolution of claw_pool). Author: kejiqing

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCapabilities {
    pub strict: bool,
    pub relaxed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxRegistration {
    #[serde(rename = "sandboxId")]
    pub sandbox_id: String,
    #[serde(rename = "sandboxBaseUrl")]
    pub sandbox_base_url: String,
    #[serde(rename = "slotsMax")]
    pub slots_max: u32,
    #[serde(rename = "slotsMin")]
    pub slots_min: u32,
    pub capabilities: SandboxCapabilities,
    #[serde(rename = "gatewayBase", skip_serializing_if = "Option::is_none")]
    pub gateway_base: Option<String>,
}
