//! Engine profiles, one per bin. Author: kejiqing

pub mod appserver;
pub mod opencode;

use std::path::PathBuf;

/// Installed engine location; the env var only redirects it for local development.
fn engine_bin(override_env: &str, default: &str) -> PathBuf {
    std::env::var(override_env)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map_or_else(|| PathBuf::from(default), PathBuf::from)
}
