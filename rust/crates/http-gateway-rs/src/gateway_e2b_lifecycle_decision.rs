//! Pure lifecycle probe → action decisions for e2b singletons and workers. Author: kejiqing
//!
//! Shared by startup reconcile, background loop, request gate, and worker acquire.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Platform + traffic probe outcome after retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeVerdict {
    /// `GET /sandboxes/{id}` not running or missing.
    NotRunning,
    /// Sandbox running and business probe succeeded.
    RunningReachable,
    /// Sandbox running but healthz/Live/traffic failed after retries.
    RunningUnreachable,
}

/// What ensure/reconcile should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleAction {
    /// Last-ok cache hit — skip network probes this tick.
    ReuseSkipProbe,
    /// Healthy — reuse/adopt; reset consecutive failure counter.
    Reuse,
    /// Kill + create (or reconcile slot).
    Recreate,
    /// Unhealthy but hysteresis — do not kill; caller returns error on request path.
    FailNoKill,
    /// Slot/sandbox busy — defer rotation (worker in_use).
    Defer,
}

/// Inputs for [`decide_lifecycle_action`].
#[derive(Debug, Clone, Copy)]
pub struct LifecycleDecisionInput {
    pub now_ms: i64,
    pub last_ok_ms: Option<i64>,
    pub consecutive_failures: u32,
    pub probe_verdict: ProbeVerdict,
    /// Admin reset or startup image pin mismatch.
    pub force_recreate: bool,
    pub busy: bool,
}

/// Short-window cache: skip probes when last success was recent.
pub const LAST_OK_CACHE_MS: i64 = 15_000;

/// Running-but-unreachable must fail this many ensure passes before recreate.
pub const CONSECUTIVE_FAIL_THRESHOLD: u32 = 2;

/// Default in-process probe retry count (sleep injected by caller).
pub const PROBE_MAX_ATTEMPTS: u32 = 3;

/// Decide lifecycle action and updated consecutive-failure count.
#[must_use]
pub fn decide_lifecycle_action(input: &LifecycleDecisionInput) -> (LifecycleAction, u32) {
    if input.busy {
        return (LifecycleAction::Defer, input.consecutive_failures);
    }

    if input.force_recreate {
        return (LifecycleAction::Recreate, 0);
    }

    if let Some(last_ok) = input.last_ok_ms {
        if input.now_ms.saturating_sub(last_ok) < LAST_OK_CACHE_MS {
            return (LifecycleAction::ReuseSkipProbe, input.consecutive_failures);
        }
    }

    match input.probe_verdict {
        ProbeVerdict::RunningReachable => (LifecycleAction::Reuse, 0),
        ProbeVerdict::NotRunning => (LifecycleAction::Recreate, 0),
        ProbeVerdict::RunningUnreachable => {
            let next = input.consecutive_failures.saturating_add(1);
            if next >= CONSECUTIVE_FAIL_THRESHOLD {
                (LifecycleAction::Recreate, 0)
            } else {
                (LifecycleAction::FailNoKill, next)
            }
        }
    }
}

/// Map combined sandbox+traffic booleans to [`ProbeVerdict`].
#[must_use]
pub fn probe_verdict_from_bools(sandbox_running: bool, traffic_reachable: bool) -> ProbeVerdict {
    if !sandbox_running {
        ProbeVerdict::NotRunning
    } else if traffic_reachable {
        ProbeVerdict::RunningReachable
    } else {
        ProbeVerdict::RunningUnreachable
    }
}

#[derive(Debug, Clone, Default)]
struct ComponentProbeState {
    last_ok_ms: Option<i64>,
    consecutive_failures: u32,
}

/// In-process probe state per component key (not shared across gateways).
#[derive(Debug, Default)]
pub struct LifecycleProbeRegistry {
    states: Mutex<HashMap<String, ComponentProbeState>>,
}

impl LifecycleProbeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            states: Mutex::new(HashMap::new()),
        }
    }

    pub fn record_success(&self, key: &str, now_ms: i64) {
        if let Ok(mut guard) = self.states.lock() {
            guard.insert(
                key.to_string(),
                ComponentProbeState {
                    last_ok_ms: Some(now_ms),
                    consecutive_failures: 0,
                },
            );
        }
    }

    pub fn snapshot(&self, key: &str) -> (Option<i64>, u32) {
        self.states
            .lock()
            .ok()
            .and_then(|g| g.get(key).cloned())
            .map(|s| (s.last_ok_ms, s.consecutive_failures))
            .unwrap_or((None, 0))
    }

    pub fn apply_decision(
        &self,
        key: &str,
        now_ms: i64,
        action: LifecycleAction,
        consecutive: u32,
    ) {
        if let Ok(mut guard) = self.states.lock() {
            let entry = guard.entry(key.to_string()).or_default();
            entry.consecutive_failures = consecutive;
            if matches!(
                action,
                LifecycleAction::Reuse | LifecycleAction::ReuseSkipProbe
            ) {
                entry.last_ok_ms = Some(now_ms);
                entry.consecutive_failures = 0;
            }
        }
    }
}

static GLOBAL_PROBE_REGISTRY: std::sync::OnceLock<Arc<LifecycleProbeRegistry>> =
    std::sync::OnceLock::new();

/// Process-wide probe registry (one per gateway process).
#[must_use]
pub fn lifecycle_probe_registry() -> Arc<LifecycleProbeRegistry> {
    GLOBAL_PROBE_REGISTRY
        .get_or_init(|| Arc::new(LifecycleProbeRegistry::new()))
        .clone()
}

pub fn singleton_probe_key(role: &str) -> String {
    format!("singleton:{role}")
}

pub fn project_observe_probe_key(proj_id: i64) -> String {
    format!("observe-proj:{proj_id}")
}

pub fn worker_slot_probe_key(proj_id: i64, slot_index: u32) -> String {
    format!("worker:{proj_id}:{slot_index}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        probe_verdict: ProbeVerdict,
        consecutive_failures: u32,
        now_ms: i64,
        last_ok_ms: Option<i64>,
    ) -> LifecycleDecisionInput {
        LifecycleDecisionInput {
            now_ms,
            last_ok_ms,
            consecutive_failures,
            probe_verdict,
            force_recreate: false,
            busy: false,
        }
    }

    #[test]
    fn last_ok_within_cache_skips_probe() {
        let (action, _) =
            decide_lifecycle_action(&input(ProbeVerdict::NotRunning, 0, 20_000, Some(10_000)));
        assert_eq!(action, LifecycleAction::ReuseSkipProbe);
    }

    #[test]
    fn last_ok_stale_probes() {
        let (action, _) =
            decide_lifecycle_action(&input(ProbeVerdict::NotRunning, 0, 30_000, Some(10_000)));
        assert_eq!(action, LifecycleAction::Recreate);
    }

    #[test]
    fn not_running_recreates_immediately() {
        let (action, cf) = decide_lifecycle_action(&input(ProbeVerdict::NotRunning, 0, 0, None));
        assert_eq!(action, LifecycleAction::Recreate);
        assert_eq!(cf, 0);
    }

    #[test]
    fn running_unreachable_first_fail_no_kill() {
        let (action, cf) =
            decide_lifecycle_action(&input(ProbeVerdict::RunningUnreachable, 0, 0, None));
        assert_eq!(action, LifecycleAction::FailNoKill);
        assert_eq!(cf, 1);
    }

    #[test]
    fn running_unreachable_second_recreate() {
        let (action, cf) =
            decide_lifecycle_action(&input(ProbeVerdict::RunningUnreachable, 1, 0, None));
        assert_eq!(action, LifecycleAction::Recreate);
        assert_eq!(cf, 0);
    }

    #[test]
    fn running_reachable_resets() {
        let (action, cf) =
            decide_lifecycle_action(&input(ProbeVerdict::RunningReachable, 2, 0, None));
        assert_eq!(action, LifecycleAction::Reuse);
        assert_eq!(cf, 0);
    }

    #[test]
    fn force_recreate_bypasses_hysteresis() {
        let mut inp = input(ProbeVerdict::RunningUnreachable, 0, 0, None);
        inp.force_recreate = true;
        let (action, _) = decide_lifecycle_action(&inp);
        assert_eq!(action, LifecycleAction::Recreate);
    }

    #[test]
    fn busy_defers_even_when_not_running() {
        let mut inp = input(ProbeVerdict::NotRunning, 0, 0, None);
        inp.busy = true;
        let (action, cf) = decide_lifecycle_action(&inp);
        assert_eq!(action, LifecycleAction::Defer);
        assert_eq!(cf, 0);
    }

    #[test]
    fn probe_verdict_from_bools_matrix() {
        assert_eq!(
            probe_verdict_from_bools(false, false),
            ProbeVerdict::NotRunning
        );
        assert_eq!(
            probe_verdict_from_bools(true, true),
            ProbeVerdict::RunningReachable
        );
        assert_eq!(
            probe_verdict_from_bools(true, false),
            ProbeVerdict::RunningUnreachable
        );
    }

    #[test]
    fn registry_records_success_and_cache() {
        let reg = LifecycleProbeRegistry::new();
        reg.record_success("singleton:nas-api", 1000);
        let (last_ok, cf) = reg.snapshot("singleton:nas-api");
        assert_eq!(last_ok, Some(1000));
        assert_eq!(cf, 0);
        let (action, _) = decide_lifecycle_action(&LifecycleDecisionInput {
            now_ms: 1014,
            last_ok_ms: last_ok,
            consecutive_failures: cf,
            probe_verdict: ProbeVerdict::NotRunning,
            force_recreate: false,
            busy: false,
        });
        assert_eq!(action, LifecycleAction::ReuseSkipProbe);
    }

    #[test]
    fn registry_apply_fail_no_kill_increments() {
        let reg = LifecycleProbeRegistry::new();
        reg.apply_decision("worker:1:0", 1000, LifecycleAction::FailNoKill, 1);
        let (_, cf) = reg.snapshot("worker:1:0");
        assert_eq!(cf, 1);
    }
}
