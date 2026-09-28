-- Project role `scope` + per-scope-key e2b workers. Author: kejiqing

ALTER TABLE project_config
    ADD COLUMN IF NOT EXISTS scope_json JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE project_config DROP CONSTRAINT IF EXISTS project_config_project_role_check;
ALTER TABLE project_config
    ADD CONSTRAINT project_config_project_role_check
    CHECK (project_role IN (
        'normal', 'master', 'observation', 'router', 'knowledge_base', 'steerable', 'scope'
    ));

ALTER TABLE project_e2b_worker
    ADD COLUMN IF NOT EXISTS scope_key TEXT NOT NULL DEFAULT '';

ALTER TABLE project_e2b_worker
    ADD COLUMN IF NOT EXISTS lifecycle_state TEXT NOT NULL DEFAULT 'running';

ALTER TABLE project_e2b_worker
    ADD COLUMN IF NOT EXISTS last_idle_at_ms BIGINT NOT NULL DEFAULT 0;

ALTER TABLE project_e2b_worker
    ADD COLUMN IF NOT EXISTS mcp_bind_json JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE project_e2b_worker DROP CONSTRAINT IF EXISTS project_e2b_worker_pkey;
ALTER TABLE project_e2b_worker
    ADD PRIMARY KEY (cluster_id, proj_id, scope_key, slot_index);

CREATE INDEX IF NOT EXISTS idx_project_e2b_worker_scope
    ON project_e2b_worker (cluster_id, proj_id, scope_key);
