//! Neuro harness: drive an ACP agent (opencode, codex-acp, …) for one gateway solve turn and speak
//! the existing worker contract (`__CLAW_GATEWAY_STDOUT__` NDJSON, `solve.done`, session jsonl).
//! Contract: `docs/neuro-harness-contract.md`. Author: kejiqing

pub mod acp_client;
pub mod cli;
pub mod engines;
pub mod mapper;
pub mod mcp_proxy;
pub mod profile;
pub mod projection;
pub mod reaper;
pub mod state;
pub mod task;
pub mod turn_context;

pub use cli::main_with_profile;
pub use profile::{AgentLaunch, EngineProfile, PrepareContext};

/// Private state directory under the session root.
pub const HARNESS_DIR: &str = ".neuro-harness";

/// Turn failure carried to `solve.done` (`clawExitCode=1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessError {
    pub status: u16,
    pub message: String,
}

impl HarnessError {
    #[must_use]
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, message)
    }
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for HarnessError {}
