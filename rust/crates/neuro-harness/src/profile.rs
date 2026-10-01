//! Engine profile: the only per-engine code (launch command + config projection). Author: kejiqing

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

pub trait EngineProfile: Sync {
    /// Value stored in `projects.harness_engine` (`opencode` | `appserver`).
    fn engine(&self) -> &'static str;

    /// Write engine config for this turn under `session_root` and return the launch command.
    fn prepare(&self, ctx: &PrepareContext<'_>) -> Result<AgentLaunch, HarnessError>;
}
