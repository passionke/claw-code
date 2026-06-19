//! FC cloud sandbox client (E2B-compatible REST + Python envd exec helper). Author: kejiqing

mod client;
mod config;
mod nas_bootstrap;
mod types;

pub use client::FcSandboxClient;
pub use config::FcSandboxConfig;
pub use nas_bootstrap::{fc_exec_with_nas_bootstrap, NAS_BOOTSTRAP_SH};
pub use types::{FcExecOutcome, FcSandboxHandle, FcSandboxVolumeMount};
