-- Per-project default agent loop max iterations (sidecar; NULL = use cluster CLAW_MAX_ITERATIONS).
-- Author: kejiqing

ALTER TABLE project_config ADD COLUMN IF NOT EXISTS max_iterations INT;
