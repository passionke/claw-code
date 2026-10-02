-- Soft-invalidate dead scope/warm workers for RCA (prefer over hard delete). Author: kejiqing
-- Live slot remains unique via existing PK; create upserts over invalid. Durable history:
-- worker_rotation_log + invalid_reason until the next successful create.

ALTER TABLE project_e2b_worker
    ADD COLUMN IF NOT EXISTS invalid_reason TEXT NOT NULL DEFAULT '';

-- Live acquire / cap / idle-pause paths filter running|sleeping.
CREATE INDEX IF NOT EXISTS idx_project_e2b_worker_live_scope
    ON project_e2b_worker (cluster_id, proj_id, scope_key, slot_index)
    WHERE lifecycle_state IN ('running', 'sleeping');

-- RCA: find currently invalid workers (stuck before recreate, or recent failures).
CREATE INDEX IF NOT EXISTS idx_project_e2b_worker_invalid_rca
    ON project_e2b_worker (cluster_id, proj_id, updated_at_ms DESC)
    WHERE lifecycle_state = 'invalid';

-- RCA: sandbox_id → lifecycle (killed vs still registered).
CREATE INDEX IF NOT EXISTS idx_project_e2b_worker_sandbox_lifecycle
    ON project_e2b_worker (sandbox_id, lifecycle_state);

-- Rotation audit already has proj/at indexes; add reason prefix lookups for invalidated/*.
CREATE INDEX IF NOT EXISTS idx_worker_rotation_log_reason_prefix
    ON worker_rotation_log (cluster_id, proj_id, reason text_pattern_ops, at_ms DESC);
