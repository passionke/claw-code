//! Engine process-tree cleanup. The solve process is the child subreaper of everything the engine
//! spawns, so orphans (codex, mcp-proxy, …) are reaped here instead of piling up as zombies under
//! the sandbox PID 1 (e2b `envd` never reaps them). Author: kejiqing

use std::time::{Duration, Instant};

/// Call before the engine is spawned.
pub fn adopt_orphans() {
    #[cfg(target_os = "linux")]
    if let Err(e) = rustix::process::set_child_subreaper(Some(rustix::process::getpid())) {
        eprintln!("[neuro-harness] PR_SET_CHILD_SUBREAPER failed: {e}");
    }
}

/// Reap every exited child (adopted orphans included). The ACP client SIGKILLs the engine process
/// group on drop, so `false` means something left that group and is still alive after `grace`.
#[must_use]
pub fn reap_children(grace: Duration) -> bool {
    use rustix::io::Errno;
    use rustix::process::{wait, WaitOptions};

    let deadline = Instant::now() + grace;
    loop {
        // `wait` is waitpid(-1): the engine runs in its own process group, which
        // `waitpid(None)` (pid 0, caller's group only) never sees.
        match wait(WaitOptions::NOHANG) {
            Ok(Some(_)) | Err(Errno::INTR) => {}
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => return false,
            Err(_) => return true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::zombie_processes)] // reaping them is what is under test
    fn reaps_exited_children_and_reports_live_ones() {
        use std::os::unix::process::CommandExt as _;

        // Own process group, like the ACP engine child.
        let done = std::process::Command::new("true")
            .process_group(0)
            .spawn()
            .unwrap();
        assert!(reap_children(Duration::from_secs(2)));
        drop(done);

        let mut live = std::process::Command::new("sleep")
            .arg("5")
            .process_group(0)
            .spawn()
            .unwrap();
        assert!(!reap_children(Duration::from_millis(50)));
        live.kill().unwrap();
        assert!(reap_children(Duration::from_secs(2)));
    }
}
