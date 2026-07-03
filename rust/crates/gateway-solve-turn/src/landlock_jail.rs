//! Per-solve Landlock self-restriction (Linux workers). Author: kejiqing

use crate::landlock_dsl::{LandlockExpandContext, ResolvedLandlockPaths};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandlockProbeStatus {
    pub supported: bool,
    pub enforcing: bool,
    pub message: String,
}

/// Probe Landlock availability on the current host.
#[must_use]
pub fn probe_landlock() -> LandlockProbeStatus {
    #[cfg(target_os = "linux")]
    {
        return probe_landlock_linux();
    }
    #[cfg(not(target_os = "linux"))]
    {
        LandlockProbeStatus {
            supported: false,
            enforcing: false,
            message: "Landlock requires Linux".into(),
        }
    }
}

#[cfg(target_os = "linux")]
fn probe_landlock_linux() -> LandlockProbeStatus {
    use landlock::{AccessFs, Compatible, Ruleset, RulesetAttr, ABI};

    match Ruleset::default()
        .set_compatibility(landlock::CompatLevel::BestEffort)
        .handle_access(AccessFs::ReadFile)
        .and_then(|ruleset| ruleset.create())
    {
        Ok(_) => LandlockProbeStatus {
            supported: true,
            enforcing: true,
            message: format!("Landlock ABI up to {:?}", ABI::V5),
        },
        Err(e) => LandlockProbeStatus {
            supported: false,
            enforcing: false,
            message: format!("Landlock probe failed: {e}"),
        },
    }
}

/// Install Landlock rules for the current process. Fail closed on strict workers.
pub fn restrict_self_landlock(paths: &ResolvedLandlockPaths) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        return restrict_self_landlock_linux(paths);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = paths;
        Err("Landlock restrict_self requires Linux worker".into())
    }
}

#[cfg(target_os = "linux")]
fn restrict_self_landlock_linux(paths: &ResolvedLandlockPaths) -> Result<(), String> {
    use landlock::{
        Access, AccessFs, Compatible, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreatedAttr, ABI,
    };

    let abi = ABI::V4;
    let write_access = AccessFs::from_all(abi);
    let read_access = AccessFs::from_read(abi);

    let mut ruleset = Ruleset::default()
        .set_compatibility(landlock::CompatLevel::BestEffort)
        .handle_access(write_access)
        .map_err(|e| format!("landlock ruleset rw handle: {e}"))?
        .handle_access(read_access)
        .map_err(|e| format!("landlock ruleset ro handle: {e}"))?
        .create()
        .map_err(|e| format!("landlock ruleset create: {e}"))?;

    for path in &paths.rw {
        let fd = PathFd::new(path).map_err(|e| format!("landlock open rw path {path}: {e}"))?;
        ruleset = ruleset
            .add_rule(PathBeneath::new(fd, write_access))
            .map_err(|e| format!("landlock add rw rule {path}: {e}"))?;
    }
    for path in resolve_ro_landlock_paths(&paths.ro) {
        let path_str = path.display().to_string();
        let fd =
            PathFd::new(&path).map_err(|e| format!("landlock open ro path {path_str}: {e}"))?;
        ruleset = ruleset
            .add_rule(PathBeneath::new(fd, read_access))
            .map_err(|e| format!("landlock add ro rule {path_str}: {e}"))?;
    }

    ruleset
        .restrict_self()
        .map_err(|e| format!("landlock_restrict_self failed (ABI {:?}): {e}", ABI::V5))?;
    Ok(())
}

/// Bootstrap strict solve: probe + expand DSL + restrict_self.
pub fn apply_strict_landlock_jail(
    dsl: &crate::landlock_dsl::LandlockDsl,
    source: crate::landlock_dsl::LandlockDslSource,
    ctx: &LandlockExpandContext<'_>,
) -> Result<(), String> {
    let probe = probe_landlock();
    if !probe.supported {
        return Err(format!(
            "strict Landlock required but unavailable: {}",
            probe.message
        ));
    }
    let paths = crate::landlock_dsl::expand_landlock_dsl(dsl, source, ctx)?;
    prepare_session_rw_dirs(&paths, ctx.session_root)?;
    restrict_self_landlock(&paths)
}

/// Resolve ro DSL paths for Landlock install. Missing optional distro paths are skipped;
/// missing `project_home_def` falls back to the `/claw_ds` mount root when present.
#[cfg(test)]
fn resolve_ro_landlock_paths(ro: &[String]) -> Vec<std::path::PathBuf> {
    use std::collections::HashSet;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for path in ro {
        for candidate in ro_landlock_candidates(path) {
            if candidate.exists() {
                let key = candidate.display().to_string();
                if seen.insert(key) {
                    out.push(candidate);
                }
            }
        }
    }
    out
}

#[cfg(test)]
fn ro_landlock_candidates(path: &str) -> Vec<std::path::PathBuf> {
    let path = path.trim();
    let p = std::path::Path::new(path);
    if p.exists() {
        return vec![p.to_path_buf()];
    }
    if path == "/claw_ds/project_home_def" || path.ends_with("/project_home_def") {
        return vec![std::path::PathBuf::from("/claw_ds")];
    }
    Vec::new()
}

fn prepare_session_rw_dirs(
    paths: &ResolvedLandlockPaths,
    session_root: &str,
) -> Result<(), String> {
    let root = std::path::Path::new(session_root);
    for raw in &paths.rw {
        let path = std::path::Path::new(raw);
        if path == root || path.starts_with(root) {
            std::fs::create_dir_all(path)
                .map_err(|e| format!("landlock mkdir rw path {raw}: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_returns_status() {
        let status = probe_landlock();
        #[cfg(not(target_os = "linux"))]
        assert!(!status.supported);
    }

    #[test]
    fn skips_missing_ro_paths_when_filtering() {
        let resolved = resolve_ro_landlock_paths(&[
            "/this-path-does-not-exist-claw-landlock-test".into(),
            "/usr".into(),
        ]);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0], std::path::PathBuf::from("/usr"));
    }

    #[test]
    fn project_home_def_missing_falls_back_to_claw_ds_mount() {
        let candidates = ro_landlock_candidates("/claw_ds/project_home_def");
        assert_eq!(candidates, vec![std::path::PathBuf::from("/claw_ds")]);
    }
}
