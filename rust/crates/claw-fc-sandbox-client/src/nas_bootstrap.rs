//! NAS staging for claw/ttyd in FC sandboxes (official code-interpreter template). Author: kejiqing

/// Shell fragment copied from `deploy/fc-sandbox/fc-nas-bootstrap.sh`.
pub const NAS_BOOTSTRAP_SH: &str =
    include_str!("../../../../deploy/fc-sandbox/fc-nas-bootstrap.sh");

/// Prefix for FC envd exec scripts (`run_sh`, solve, ttyd start).
#[must_use]
pub fn fc_exec_with_nas_bootstrap(user_script: &str, tools_rel: &str) -> String {
    format!(
        "export CLAW_FC_NAS_TOOLS_REL={tools_rel:?}\n{NAS_BOOTSTRAP_SH}\n{user_script}",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_contains_tools_rel_export() {
        let s = fc_exec_with_nas_bootstrap("echo ok", ".claw-fc-tools");
        assert!(s.contains("CLAW_FC_NAS_TOOLS_REL=\".claw-fc-tools\""));
        assert!(s.contains("echo ok"));
        assert!(s.contains("fc_tools_src"));
    }
}
