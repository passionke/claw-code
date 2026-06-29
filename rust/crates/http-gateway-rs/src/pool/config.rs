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
