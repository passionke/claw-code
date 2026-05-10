//! Types shared by pool backends. Author: kejiqing

/// Lease for one worker slot (index into the pool).
#[derive(Debug, Clone)]
pub struct SlotLease {
    pub slot_index: usize,
}

/// Result of `docker exec` (or equivalent) running `claw gateway-solve-once`.
#[derive(Debug, Clone)]
pub struct TaskOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}
