//! Worker exec user for isolation mode. Author: kejiqing

use claw_sandbox_protocol::{GuestExecActor, IsolationMode, SlotExecIdentity};

use super::worker_identity::PoolWorkerIdentity;

#[must_use]
pub fn exec_user_for_isolation(
    mode: IsolationMode,
    worker_identity: &PoolWorkerIdentity,
) -> String {
    match mode {
        IsolationMode::Relaxed => "0:0".to_string(),
        IsolationMode::Strict => worker_identity.exec_user_arg(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use claw_sandbox_protocol::IsolationMode;

    use super::super::worker_identity::PoolWorkerIdentity;

    #[test]
    fn strict_exec_uses_pool_worker() {
        let id = PoolWorkerIdentity::from_env(None);
        assert_eq!(
            exec_user_for_isolation(IsolationMode::Strict, &id),
            id.exec_user_arg()
        );
    }

    #[test]
    fn relaxed_exec_is_root() {
        let id = PoolWorkerIdentity::from_env(None);
        assert_eq!(
            exec_user_for_isolation(IsolationMode::Relaxed, &id),
            "0:0"
        );
    }
}
