//! Engine profile: the only per-engine code (launch command + config projection). Author: kejiqing

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agent_client_protocol::schema::v1::SessionUpdate;

use crate::HarnessError;

/// Inputs a profile may project into its engine-specific config.
#[derive(Debug, Clone)]
pub struct PrepareContext<'a> {
    pub session_root: &'a Path,
    /// `CLAW_PROJECT_CONFIG_ROOT` (`project_home_def`).
    pub project_config_root: &'a Path,
    /// Wire model id sent to the tap.
    pub model: &'a str,
    /// `OPENAI_BASE_URL` (tap proxy, path passthrough).
    pub openai_base_url: &'a str,
    /// Rendered `.neuro-harness/instructions.md`.
    pub instructions_path: &'a Path,
    /// `$CLAW_PROJECT_CONFIG_ROOT/.claw/skills`.
    pub skills_dir: &'a Path,
}

/// How to spawn the ACP agent. `env` is added on top of the inherited process env.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunch {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// Engine-private turn state carried in-band on ACP updates (e.g. codex `_meta`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnSignal {
    /// Latest upstream error detail; becomes the failure message.
    ErrorDetail(String),
    /// The engine gave up on the turn, even if the prompt still ends with `end_turn`.
    Failed,
}

pub trait EngineProfile: Sync {
    /// Value stored in `projects.harness_engine` (`opencode` | `appserver`).
    fn engine(&self) -> &'static str;

    /// Write engine config for this turn under `session_root` and return the launch command.
    fn prepare(&self, ctx: &PrepareContext<'_>) -> Result<AgentLaunch, HarnessError>;

    /// In-band signal on an update; engines that fail through ACP errors keep the default.
    fn turn_signal(&self, _update: &SessionUpdate) -> Option<TurnSignal> {
        None
    }
}
