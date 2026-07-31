//! Live Background sub-agent registry for same-turn Await + turn-end kill. Author: kejiqing

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use runtime::HookAbortSignal;

/// Process-wide live Background agents for the current worker (one solve turn at a time). Author: kejiqing
static LIVE: Mutex<Option<LiveAgentRegistry>> = Mutex::new(None);

#[derive(Clone)]
pub struct LiveAgentHandle {
    pub agent_id: String,
    pub abort: HookAbortSignal,
    pub done: Arc<AtomicBool>,
    finished: Arc<(Mutex<bool>, Condvar)>,
}

struct LiveAgentEntry {
    handle: LiveAgentHandle,
}

pub struct LiveAgentRegistry {
    agents: HashMap<String, LiveAgentEntry>,
}

impl LiveAgentRegistry {
    fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }
}

fn with_registry<R>(f: impl FnOnce(&mut LiveAgentRegistry) -> R) -> R {
    let mut guard = LIVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_none() {
        *guard = Some(LiveAgentRegistry::new());
    }
    f(guard.as_mut().expect("registry initialized"))
}

/// Register a Background agent before its thread starts. Author: kejiqing
pub fn register_live_agent(agent_id: impl Into<String>) -> LiveAgentHandle {
    let agent_id = agent_id.into();
    let handle = LiveAgentHandle {
        agent_id: agent_id.clone(),
        abort: HookAbortSignal::new(),
        done: Arc::new(AtomicBool::new(false)),
        finished: Arc::new((Mutex::new(false), Condvar::new())),
    };
    with_registry(|reg| {
        reg.agents.insert(
            agent_id,
            LiveAgentEntry {
                handle: handle.clone(),
            },
        );
    });
    handle
}

/// Mark agent finished and wake Await waiters. Author: kejiqing
pub fn mark_live_agent_finished(agent_id: &str) {
    let finished = with_registry(|reg| {
        reg.agents.get(agent_id).map(|e| {
            e.handle.done.store(true, Ordering::SeqCst);
            Arc::clone(&e.handle.finished)
        })
    });
    if let Some(finished) = finished {
        let (lock, cvar) = &*finished;
        let mut done = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *done = true;
        cvar.notify_all();
    }
    with_registry(|reg| {
        reg.agents.remove(agent_id);
    });
}

/// How many Background agents are still registered as live. Author: kejiqing
#[must_use]
pub fn live_agent_count() -> usize {
    with_registry(|reg| reg.agents.len())
}

/// True if `agent_id` is still in the live registry. Author: kejiqing
#[must_use]
pub fn is_live_agent(agent_id: &str) -> bool {
    with_registry(|reg| reg.agents.contains_key(agent_id))
}

/// Wait until agent finishes or timeout. Returns true if finished. Author: kejiqing
pub fn wait_live_agent(agent_id: &str, timeout: Duration) -> bool {
    let finished = with_registry(|reg| {
        reg.agents
            .get(agent_id)
            .map(|e| Arc::clone(&e.handle.finished))
    });
    let Some(finished) = finished else {
        // Already removed ⇒ finished (or never registered / already awaited).
        return true;
    };
    let (lock, cvar) = &*finished;
    let guard = lock
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *guard {
        return true;
    }
    let (guard, wait_result) = cvar
        .wait_timeout(guard, timeout)
        .unwrap_or_else(|e| e.into_inner());
    *guard || !wait_result.timed_out() && *guard
}

/// Abort all live agents and wait until registry is empty (or timeout). Author: kejiqing
pub fn kill_all_subagents(join_timeout: Duration) -> usize {
    let handles: Vec<LiveAgentHandle> = with_registry(|reg| {
        reg.agents
            .values()
            .map(|e| e.handle.clone())
            .collect::<Vec<_>>()
    });
    let remaining_before = handles.len();
    for h in &handles {
        h.abort.abort();
    }
    let deadline = Instant::now() + join_timeout;
    for h in &handles {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let _ = wait_live_agent(&h.agent_id, left);
    }
    // Force-clear any stragglers so the registry reports zero (thread may still unwind).
    with_registry(|reg| {
        for (id, entry) in reg.agents.drain() {
            entry.handle.done.store(true, Ordering::SeqCst);
            let (lock, cvar) = &*entry.handle.finished;
            let mut done = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *done = true;
            cvar.notify_all();
            let _ = id;
        }
    });
    remaining_before
}

/// Reset registry (tests). Author: kejiqing
pub fn reset_live_agents_for_test() {
    with_registry(|reg| {
        reg.agents.clear();
    });
}

/// Serialize tests that mutate the process-wide live registry. Author: kejiqing
pub fn live_agent_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn kill_all_clears_registry_after_cooperative_finish() {
        let _guard = live_agent_test_lock();
        reset_live_agents_for_test();
        let handle = register_live_agent("kill-test-1");
        let abort = handle.abort.clone();
        let agent_id = handle.agent_id.clone();
        let t = thread::spawn(move || {
            while !abort.is_aborted() {
                thread::sleep(Duration::from_millis(10));
            }
            mark_live_agent_finished(&agent_id);
        });
        assert_eq!(live_agent_count(), 1);
        let n = kill_all_subagents(Duration::from_secs(2));
        assert_eq!(n, 1);
        assert_eq!(live_agent_count(), 0);
        t.join().expect("worker");
        reset_live_agents_for_test();
    }

    #[test]
    fn double_kill_all_is_idempotent() {
        let _guard = live_agent_test_lock();
        reset_live_agents_for_test();
        assert_eq!(kill_all_subagents(Duration::from_millis(50)), 0);
        assert_eq!(kill_all_subagents(Duration::from_millis(50)), 0);
        reset_live_agents_for_test();
    }

    #[test]
    fn escape_detector_sees_unkilled_child() {
        let _guard = live_agent_test_lock();
        reset_live_agents_for_test();
        let handle = register_live_agent("escape-detect");
        assert!(is_live_agent("escape-detect"));
        assert_eq!(live_agent_count(), 1);
        handle.abort.abort();
        mark_live_agent_finished("escape-detect");
        assert_eq!(live_agent_count(), 0);
        reset_live_agents_for_test();
    }
}
