//! Integration tests: turn-end kill leaves no live Background sub-agents. Author: kejiqing

use std::time::Duration;

use tools::{
    is_live_agent, kill_all_subagents, live_agent_count, live_agent_test_lock,
    mark_live_agent_finished, register_live_agent, reset_live_agents_for_test,
};

#[test]
fn integ_background_child_dead_after_solve_returns() {
    let _guard = live_agent_test_lock();
    reset_live_agents_for_test();
    let handle = register_live_agent("integ-bg-1");
    let abort = handle.abort.clone();
    let agent_id = handle.agent_id.clone();
    let worker = std::thread::spawn(move || {
        while !abort.is_aborted() {
            std::thread::sleep(Duration::from_millis(5));
        }
        mark_live_agent_finished(&agent_id);
    });
    assert_eq!(live_agent_count(), 1);
    // Simulate run_gateway_solve_turn teardown.
    let killed = kill_all_subagents(Duration::from_secs(2));
    assert_eq!(killed, 1);
    assert_eq!(live_agent_count(), 0);
    assert!(!is_live_agent("integ-bg-1"));
    worker.join().expect("worker");
    reset_live_agents_for_test();
}

#[test]
fn integ_second_turn_no_orphan_from_first() {
    let _guard = live_agent_test_lock();
    reset_live_agents_for_test();
    let handle = register_live_agent("integ-orphan");
    let abort = handle.abort.clone();
    let agent_id = handle.agent_id.clone();
    let worker = std::thread::spawn(move || {
        while !abort.is_aborted() {
            std::thread::sleep(Duration::from_millis(5));
        }
        mark_live_agent_finished(&agent_id);
    });
    let _ = kill_all_subagents(Duration::from_secs(2));
    worker.join().expect("worker");
    assert_eq!(live_agent_count(), 0);
    // Second turn starts clean.
    assert!(!is_live_agent("integ-orphan"));
    reset_live_agents_for_test();
}

#[test]
fn integ_abort_mid_background_no_escape() {
    let _guard = live_agent_test_lock();
    reset_live_agents_for_test();
    let handle = register_live_agent("integ-abort");
    let abort = handle.abort.clone();
    let agent_id = handle.agent_id.clone();
    let worker = std::thread::spawn(move || {
        while !abort.is_aborted() {
            std::thread::sleep(Duration::from_millis(5));
        }
        mark_live_agent_finished(&agent_id);
    });
    let killed = kill_all_subagents(Duration::from_secs(2));
    assert_eq!(killed, 1);
    assert_eq!(live_agent_count(), 0);
    worker.join().expect("worker");
    reset_live_agents_for_test();
}
