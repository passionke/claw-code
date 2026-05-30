//! Pool worker OS identity: exec user, pkill target, uid/gid aligned with chown. Author: kejiqing

use crate::workspace_perm;

/// Resolved once at pool construction; shared by exec, chown, and release cleanup.
#[derive(Debug, Clone)]
pub struct PoolWorkerIdentity {
    pub uid: u32,
    pub gid: u32,
    /// Login name for `pkill -u` and optional named `docker exec --user`.
    pub exec_user: String,
    /// When true, `exec` uses `--user exec_user`; otherwise `--user uid:gid`.
    pub use_named_exec_user: bool,
}

impl PoolWorkerIdentity {
    /// `CLAW_*_POOL_EXEC_USER` overrides name; default exec uses `uid:gid` (not container root).
    #[must_use]
    pub fn from_env(exec_user_override: Option<String>) -> Self {
        let (uid, gid) = workspace_perm::worker_uid_gid();
        let named = exec_user_override
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let use_named_exec_user = named.is_some();
        let exec_user = named.unwrap_or_else(|| "claw".to_string());
        Self {
            uid,
            gid,
            exec_user,
            use_named_exec_user,
        }
    }

    /// Argument to `docker exec --user`.
    #[must_use]
    pub fn exec_user_arg(&self) -> String {
        if self.use_named_exec_user {
            self.exec_user.clone()
        } else {
            format!("{}:{}", self.uid, self.gid)
        }
    }

    #[must_use]
    pub fn pkill_user(&self) -> &str {
        self.exec_user.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_uses_uid_gid_not_name() {
        let id = PoolWorkerIdentity::from_env(None);
        assert!(!id.use_named_exec_user);
        assert_eq!(id.exec_user_arg(), format!("{}:{}", id.uid, id.gid));
    }

    #[test]
    fn named_exec_user_when_set() {
        let id = PoolWorkerIdentity::from_env(Some("clawUser".into()));
        assert!(id.use_named_exec_user);
        assert_eq!(id.exec_user_arg(), "clawUser");
        assert_eq!(id.pkill_user(), "clawUser");
    }

    #[test]
    fn pkill_user_matches_exec_user_not_hardcoded_root() {
        let id = PoolWorkerIdentity::from_env(None);
        assert_eq!(id.pkill_user(), "claw");
        assert_ne!(id.pkill_user(), "root");
        assert_ne!(id.exec_user_arg(), "root");
    }
}
