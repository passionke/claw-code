-- Per-project FC worker sandbox registry (gateway-managed lifecycle). Author: kejiqing
CREATE TABLE IF NOT EXISTS project_fc_worker (
    proj_id BIGINT PRIMARY KEY,
    sandbox_id TEXT NOT NULL,
    worker_id TEXT NOT NULL,
    template_id TEXT NOT NULL,
    handle_json JSONB NOT NULL,
    updated_at_ms BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_project_fc_worker_sandbox_id ON project_fc_worker (sandbox_id);
