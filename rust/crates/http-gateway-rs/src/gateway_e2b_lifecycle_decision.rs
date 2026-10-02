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

// --- Scope worker acquire (dead → soft-invalidate → create) ---
// Author: kejiqing
//
// Warm pool uses [`decide_lifecycle_action`] (recreate / hysteresis). Scope workers have no warm
// reconcile: a dead or unresumable sandbox must soft-invalidate the PG row
// (`lifecycle_state=invalid` + `invalid_reason`) so the next path creates fresh. Hard-failing with
// "sandbox not running" while the row remains live is a regression (FDA / pre).
// Durable RCA: `worker_rotation_log` + `invalid_reason` (until create upserts over the same PK).

/// Platform probe for one scope worker sandbox (caller supplies booleans).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeSandboxProbe {
    Running,
    Paused,
    /// killed / missing / any non-running non-paused state.
    Dead,
}

/// What [`acquire_for_scope_solve`] / ensure should do for an existing scope row or cache entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeWorkerAction {
    /// Sandbox is running — reuse handle.
    Reuse,
    /// Must `resume_sandbox`; on failure apply [`ScopeWorkerAction::Invalidate`].
    Resume,
    /// Soft-invalidate PG row (`lifecycle_state=invalid`), clear cache, then create.
    Invalidate,
}

/// Map e2b paused/running flags to [`ScopeSandboxProbe`]. Paused wins if both true.
#[must_use]
pub fn scope_sandbox_probe(paused: bool, running: bool) -> ScopeSandboxProbe {
    if paused {
        ScopeSandboxProbe::Paused
    } else if running {
        ScopeSandboxProbe::Running
    } else {
        ScopeSandboxProbe::Dead
    }
}

/// Decide action for an existing scope worker (PG row or warm cache).
///
/// Unlike warm [`decide_lifecycle_action`]:
/// - `Dead` → [`ScopeWorkerAction::Invalidate`] immediately (no last-ok skip, no FailNoKill).
/// - PG `sleeping` + platform `Dead` → invalidate (do not call resume on a corpse).
/// - PG already `invalid` → invalidate again is a no-op path (caller treats as missing).
#[must_use]
pub fn decide_scope_existing_worker(
    pg_lifecycle_state: &str,
    probe: ScopeSandboxProbe,
) -> ScopeWorkerAction {
    if pg_lifecycle_state == "invalid" {
        return ScopeWorkerAction::Invalidate;
    }
    let want_resume =
        pg_lifecycle_state == "sleeping" || matches!(probe, ScopeSandboxProbe::Paused);
    if want_resume {
        return match probe {
            ScopeSandboxProbe::Paused | ScopeSandboxProbe::Running => ScopeWorkerAction::Resume,
            ScopeSandboxProbe::Dead => ScopeWorkerAction::Invalidate,
        };
    }
    match probe {
        ScopeSandboxProbe::Running => ScopeWorkerAction::Reuse,
        ScopeSandboxProbe::Paused => ScopeWorkerAction::Resume,
        ScopeSandboxProbe::Dead => ScopeWorkerAction::Invalidate,
    }
}

/// Cache-only probe (no PG lifecycle): running reuse, paused resume, else invalidate.
#[must_use]
pub fn decide_scope_probe_only(probe: ScopeSandboxProbe) -> ScopeWorkerAction {
    decide_scope_existing_worker("running", probe)
}

/// Resume attempt failed (HTTP 500 / missing docker container / etc.) → invalidate.
#[must_use]
pub fn decide_scope_after_resume_failure() -> ScopeWorkerAction {
    ScopeWorkerAction::Invalidate
}

/// Stable detail codes for logs / `worker_rotation_log.reason` / `invalid_reason`.
#[must_use]
pub fn scope_drop_detail(pg_lifecycle_state: &str, probe: ScopeSandboxProbe) -> &'static str {
    match (pg_lifecycle_state == "sleeping", probe) {
        (true, ScopeSandboxProbe::Dead) => "pg_sleeping_but_dead",
        (_, ScopeSandboxProbe::Dead) => "not_running",
        (true, _) => "resume_required_but_unusable",
        _ => "not_running",
    }
}

/// Audit / `invalid_reason` fragment: `invalidated;{detail};scope_key={scope_key}`.
#[must_use]
pub fn scope_invalidate_audit_reason(detail: &str, scope_key: &str) -> String {
    format!("invalidated;{detail};scope_key={scope_key}")
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

    // --- Scope dead-as-missing (regression matrix). Author: kejiqing ---

    #[test]
    fn scope_probe_paused_wins_over_running() {
        assert_eq!(
            scope_sandbox_probe(true, true),
            ScopeSandboxProbe::Paused
        );
        assert_eq!(
            scope_sandbox_probe(false, true),
            ScopeSandboxProbe::Running
        );
        assert_eq!(scope_sandbox_probe(false, false), ScopeSandboxProbe::Dead);
        assert_eq!(scope_sandbox_probe(true, false), ScopeSandboxProbe::Paused);
    }

    /// FDA / pre: PG still `running` while e2b `killed` must drop, not hard-fail.
    #[test]
    fn scope_fda_pg_running_e2b_killed_drops() {
        let action =
            decide_scope_existing_worker("running", ScopeSandboxProbe::Dead);
        assert_eq!(action, ScopeWorkerAction::Invalidate);
        assert_eq!(
            scope_drop_detail("running", ScopeSandboxProbe::Dead),
            "not_running"
        );
    }

    #[test]
    fn scope_pg_running_e2b_running_reuses() {
        assert_eq!(
            decide_scope_existing_worker("running", ScopeSandboxProbe::Running),
            ScopeWorkerAction::Reuse
        );
    }

    #[test]
    fn scope_pg_running_e2b_paused_resumes() {
        assert_eq!(
            decide_scope_existing_worker("running", ScopeSandboxProbe::Paused),
            ScopeWorkerAction::Resume
        );
    }

    #[test]
    fn scope_pg_sleeping_e2b_paused_resumes() {
        assert_eq!(
            decide_scope_existing_worker("sleeping", ScopeSandboxProbe::Paused),
            ScopeWorkerAction::Resume
        );
    }

    /// Resume docker-container-missing (sbx_7a4cdb…) → drop as missing, not sticky 503.
    #[test]
    fn scope_resume_failure_always_drops() {
        assert_eq!(
            decide_scope_after_resume_failure(),
            ScopeWorkerAction::Invalidate
        );
    }

    /// PG sleeping but platform already dead: do not attempt resume.
    #[test]
    fn scope_pg_sleeping_e2b_dead_drops_without_resume() {
        assert_eq!(
            decide_scope_existing_worker("sleeping", ScopeSandboxProbe::Dead),
            ScopeWorkerAction::Invalidate
        );
        assert_eq!(
            scope_drop_detail("sleeping", ScopeSandboxProbe::Dead),
            "pg_sleeping_but_dead"
        );
    }

    /// PG sleeping + somehow still running → resume path (match prior need_resume semantics).
    #[test]
    fn scope_pg_sleeping_e2b_running_still_resumes() {
        assert_eq!(
            decide_scope_existing_worker("sleeping", ScopeSandboxProbe::Running),
            ScopeWorkerAction::Resume
        );
    }

    #[test]
    fn scope_cache_probe_only_matches_running_lifecycle() {
        assert_eq!(
            decide_scope_probe_only(ScopeSandboxProbe::Dead),
            decide_scope_existing_worker("running", ScopeSandboxProbe::Dead)
        );
        assert_eq!(
            decide_scope_probe_only(ScopeSandboxProbe::Paused),
            ScopeWorkerAction::Resume
        );
        assert_eq!(
            decide_scope_probe_only(ScopeSandboxProbe::Running),
            ScopeWorkerAction::Reuse
        );
    }

    /// Scope must not inherit warm FailNoKill hysteresis on Dead.
    #[test]
    fn scope_dead_never_fail_no_kill_unlike_warm_unreachable() {
        let (warm, _) =
            decide_lifecycle_action(&input(ProbeVerdict::RunningUnreachable, 0, 0, None));
        assert_eq!(warm, LifecycleAction::FailNoKill);

        let scope = decide_scope_existing_worker("running", ScopeSandboxProbe::Dead);
        assert_eq!(scope, ScopeWorkerAction::Invalidate);
        assert_ne!(
            format!("{scope:?}"),
            format!("{:?}", LifecycleAction::FailNoKill)
        );
    }

    /// Warm NotRunning → Recreate; scope Dead → Invalidate (create after row delete).
    #[test]
    fn scope_vs_warm_not_running_asymmetry_documented() {
        let (warm, _) = decide_lifecycle_action(&input(ProbeVerdict::NotRunning, 0, 0, None));
        assert_eq!(warm, LifecycleAction::Recreate);
        assert_eq!(
            decide_scope_existing_worker("running", ScopeSandboxProbe::Dead),
            ScopeWorkerAction::Invalidate
        );
    }

    /// Stale last-ok must not skip drop for scope (warm can ReuseSkipProbe).
    #[test]
    fn scope_dead_ignores_warm_last_ok_cache_semantics() {
        let (warm_skip, _) =
            decide_lifecycle_action(&input(ProbeVerdict::NotRunning, 0, 20_000, Some(10_000)));
        assert_eq!(warm_skip, LifecycleAction::ReuseSkipProbe);
        // Scope decision has no last_ok input — always Invalidate when Dead.
        assert_eq!(
            decide_scope_existing_worker("running", ScopeSandboxProbe::Dead),
            ScopeWorkerAction::Invalidate
        );
    }

    #[test]
    fn scope_audit_reason_format_stable() {
        let scope_key =
            "tenantId=27c56517-d1f2-44db-ba67-0884583c6253\u{1f}uid=f60afc97-e1c7-4c1f-b600-51eea62db1d5";
        let reason = scope_invalidate_audit_reason("not_running", scope_key);
        assert!(reason.starts_with("invalidated;not_running;scope_key="));
        assert!(reason.contains(scope_key));
        let resume = scope_invalidate_audit_reason("resume_failed:http500", scope_key);
        assert!(resume.contains("resume_failed:http500"));
    }

    #[test]
    fn scope_drop_is_create_path_not_hard_error_token() {
        // Old bug string must never be the decision outcome — acquire must create after drop.
        let action = decide_scope_existing_worker("running", ScopeSandboxProbe::Dead);
        assert_eq!(action, ScopeWorkerAction::Invalidate);
        assert!(!matches!(action, ScopeWorkerAction::Reuse));
    }

    /// Exhaustive (pg_lifecycle × probe) matrix — catches future regressions on scope acquire.
    #[test]
    fn scope_existing_worker_full_matrix() {
        use ScopeSandboxProbe::*;
        use ScopeWorkerAction::*;
        let cases: &[(&str, ScopeSandboxProbe, ScopeWorkerAction)] = &[
            ("running", Running, Reuse),
            ("running", Paused, Resume),
            ("running", Dead, Invalidate),
            ("sleeping", Running, Resume),
            ("sleeping", Paused, Resume),
            ("sleeping", Dead, Invalidate),
            // Unknown lifecycle treated like non-sleeping.
            ("unknown", Running, Reuse),
            ("unknown", Paused, Resume),
            ("unknown", Dead, Invalidate),
            ("", Running, Reuse),
            ("", Paused, Resume),
            ("", Dead, Invalidate),
            // Already soft-invalidated: treat as missing (idempotent invalidate / create).
            ("invalid", Running, Invalidate),
            ("invalid", Paused, Invalidate),
            ("invalid", Dead, Invalidate),
        ];
        for (lifecycle, probe, expected) in cases {
            assert_eq!(
                decide_scope_existing_worker(lifecycle, *probe),
                *expected,
                "lifecycle={lifecycle:?} probe={probe:?}"
            );
        }
    }
}
