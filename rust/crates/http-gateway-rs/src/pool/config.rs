//! Gateway pool env helpers. Author: kejiqing

/// `CLAW_ALLOW_RELAXED_WORKER` — when false, all ds use strict profile metadata. Author: kejiqing
#[must_use]
pub fn relaxed_worker_allowed_from_env() -> bool {
    match std::env::var("CLAW_ALLOW_RELAXED_WORKER") {
        Ok(v) => {
            let t = v.trim().to_ascii_lowercase();
            !(t == "0" || t == "false" || t == "no" || t == "off")
        }
        Err(_) => true,
    }
}

#[cfg(test)]
pub(crate) fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaxed_allowed_defaults_true_when_unset() {
        let _g = test_env_lock();
        let prev = std::env::var("CLAW_ALLOW_RELAXED_WORKER").ok();
        std::env::remove_var("CLAW_ALLOW_RELAXED_WORKER");
        assert!(relaxed_worker_allowed_from_env());
        match prev {
            Some(v) => std::env::set_var("CLAW_ALLOW_RELAXED_WORKER", v),
            None => std::env::remove_var("CLAW_ALLOW_RELAXED_WORKER"),
        }
    }

    #[test]
    fn relaxed_disallowed_for_falsey_values() {
        let _g = test_env_lock();
        let prev = std::env::var("CLAW_ALLOW_RELAXED_WORKER").ok();
        for v in ["0", "false", "FALSE", "no", "off"] {
            std::env::set_var("CLAW_ALLOW_RELAXED_WORKER", v);
            assert!(
                !relaxed_worker_allowed_from_env(),
                "expected disallowed for {v}"
            );
        }
        match prev {
            Some(v) => std::env::set_var("CLAW_ALLOW_RELAXED_WORKER", v),
            None => std::env::remove_var("CLAW_ALLOW_RELAXED_WORKER"),
        }
    }
}
