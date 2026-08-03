-- Per-project worker env map injected only at e2b warm-proj create (sidecar, not in revision).
-- Author: kejiqing

ALTER TABLE project_config ADD COLUMN IF NOT EXISTS worker_env_json JSONB NOT NULL DEFAULT '{}'::jsonb;
