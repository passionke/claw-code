//! Worker exec user for isolation mode. Author: kejiqing

use claw_sandbox_protocol::{GuestExecActor, IsolationMode, SlotExecIdentity};

use super::worker_identity::PoolWorkerIdentity;

#[must_use]
pub fn exec_user_for_isolation(
    mode: IsolationMode,
    worker_identity: &PoolWorkerIdentity,
) -> String {
    // strict and relaxed both solve as pool worker user (`claw`); isolation differs by image/caps. Author: kejiqing
    let _ = mode;
    worker_identity.exec_user_arg()
}

#[must_use]
pub fn slot_exec_identity(
    isolation: IsolationMode,
    worker_identity: &PoolWorkerIdentity,
) -> SlotExecIdentity {
    SlotExecIdentity {
        isolation,
        exec_user: exec_user_for_isolation(isolation, worker_identity),
    }
}

#[must_use]
pub fn exec_user_for_actor(
    actor: GuestExecActor,
    isolation: IsolationMode,
    worker_identity: &PoolWorkerIdentity,
) -> String {
    match actor {
        GuestExecActor::PoolRoot => "0:0".to_string(),
        GuestExecActor::SlotWorker => exec_user_for_isolation(isolation, worker_identity),
    }
}
