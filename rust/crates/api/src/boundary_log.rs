//! Optional provider/runtime trace lines (`[runtime-boundary]`, `[boundary-out]`, …).
//! Off by default so normal `claw` runs keep a clean console; enable for debugging. kejiqing

/// Process env: set to `1`, `true`, `yes`, or `on` to print boundary trace lines to stderr.
pub const BOUNDARY_LOG_ENV: &str = "CLAW_BOUNDARY_LOG";

#[must_use]
pub fn boundary_log_enabled() -> bool {
    match std::env::var(BOUNDARY_LOG_ENV) {
        Ok(value) => {
            let v = value.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    use super::boundary_log_enabled;
    use super::BOUNDARY_LOG_ENV;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Snapshot-restore a single env var (same pattern as `providers::mod::tests`).
    struct EnvVarGuard {
        key: &'static str,
        original: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: Option<&str>) -> Self {
            let original = std::env::var_os(key);
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
            Self { key, original }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.original.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn boundary_log_is_off_without_env() {
        let _lock = env_lock();
        let _guard = EnvVarGuard::set(BOUNDARY_LOG_ENV, None);
        assert!(!boundary_log_enabled());
    }

    #[test]
    fn boundary_log_is_on_for_explicit_values() {
        let _lock = env_lock();
        let _guard = EnvVarGuard::set(BOUNDARY_LOG_ENV, Some("1"));
        assert!(boundary_log_enabled());
    }
}
