//! Worker report SSE timing: model trunk in → hub coalesce → SSE wire → gateway proxy. Author: kejiqing

use std::sync::atomic::{AtomicBool, Ordering};

use tracing::info;

#[cfg(test)]
static FORCE_ENABLED: AtomicBool = AtomicBool::new(false);

/// `CLAW_REPORT_SSE_TIMING=1` or `CLAW_SSE_DEBUG=1`. Author: kejiqing
#[must_use]
pub fn enabled() -> bool {
    #[cfg(test)]
    if FORCE_ENABLED.load(Ordering::SeqCst) {
        return true;
    }
    fn on(name: &str) -> bool {
        std::env::var(name).ok().is_some_and(|v| {
            let s = v.trim().to_ascii_lowercase();
            matches!(s.as_str(), "1" | "true" | "yes" | "on")
        })
    }
    on("CLAW_REPORT_SSE_TIMING") || on("CLAW_SSE_DEBUG")
}

#[cfg(test)]
pub fn force_enabled_for_test() {
    FORCE_ENABLED.store(true, Ordering::SeqCst);
}

#[must_use]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[must_use]
pub fn lag_ms(later_ms: i64, earlier_ms: i64) -> i64 {
    (later_ms - earlier_ms).max(0)
}

pub fn log_trunk_in(turn_id: &str, chars: usize) {
    if !enabled() {
        return;
    }
    let at = now_ms();
    info!(
        target: "claw_report_sse_timing",
        component = "worker_report_sse",
        phase = "trunk_in",
        turn_id = %turn_id,
        trunk_at_ms = at,
        chars,
    );
}

pub fn log_hub_push(turn_id: &str, trunk_first_ms: i64, hub_push_ms: i64, chars: usize, chunk_idx: usize) {
    if !enabled() {
        return;
    }
    info!(
        target: "claw_report_sse_timing",
        component = "worker_report_sse",
        phase = "hub_push",
        turn_id = %turn_id,
        chunk_idx,
        trunk_first_ms,
        hub_push_ms,
        trunk_to_hub_ms = lag_ms(hub_push_ms, trunk_first_ms),
        chars,
    );
}

pub fn log_sse_emit(
    turn_id: &str,
    trunk_first_ms: i64,
    hub_push_ms: i64,
    sse_emit_ms: i64,
    chars: usize,
    chunk_idx: usize,
    subscriber_idx: u64,
) {
    if !enabled() {
        return;
    }
    info!(
        target: "claw_report_sse_timing",
        component = "worker_report_sse",
        phase = "sse_emit",
        turn_id = %turn_id,
        chunk_idx,
        subscriber_idx,
        trunk_first_ms,
        hub_push_ms,
        sse_emit_ms,
        trunk_to_hub_ms = lag_ms(hub_push_ms, trunk_first_ms),
        hub_to_sse_ms = lag_ms(sse_emit_ms, hub_push_ms),
        trunk_to_sse_ms = lag_ms(sse_emit_ms, trunk_first_ms),
        chars,
    );
}

pub fn log_sse_subscriber_open(turn_id: &str, subscriber_idx: u64) {
    if !enabled() {
        return;
    }
    info!(
        target: "claw_report_sse_timing",
        component = "worker_report_sse",
        phase = "sse_subscriber_open",
        turn_id = %turn_id,
        subscriber_idx,
        at_ms = now_ms(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lag_ms_non_negative() {
        assert_eq!(lag_ms(100, 40), 60);
        assert_eq!(lag_ms(10, 50), 0);
    }
}
