//! Stable pool identity (`CLAW_POOL_ID` or `pool-{hostname}`). Author: kejiqing

/// Stable pool identity per machine (`CLAW_POOL_ID` or `pool-{hostname}`). Author: kejiqing
#[must_use]
pub fn resolve_pool_id() -> String {
    if let Ok(v) = std::env::var("CLAW_POOL_ID") {
        let t = v.trim();
        if !t.is_empty() {
            return t.to_string();
        }
    }
    let host = std::env::var("HOSTNAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".to_string());
    format!("pool-{host}")
}
