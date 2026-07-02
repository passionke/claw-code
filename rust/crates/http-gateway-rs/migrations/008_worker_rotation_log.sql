-- Append-only audit log of per-project e2b worker rotations. Author: kejiqing
-- Separate from project_e2b_worker (current-state pointer): this table is history only,
-- never updated/deleted, so worker create/rotate is traceable without touching the state row.
CREATE TABLE IF NOT EXISTS worker_rotation_log (
    id BIGSERIAL PRIMARY KEY,
    proj_id BIGINT NOT NULL,
    event TEXT NOT NULL,
    sandbox_id TEXT,
    worker_id TEXT,
    template_id TEXT,
    reason TEXT,
    at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_worker_rotation_log_proj
    ON worker_rotation_log (proj_id, at_ms DESC);
