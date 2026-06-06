-- Soft-archive sessions (hidden from default sidebar list). Author: kejiqing

ALTER TABLE claw_sessions ADD COLUMN IF NOT EXISTS archived_at_ms BIGINT;

CREATE INDEX IF NOT EXISTS idx_claw_sessions_user_project_active
  ON claw_sessions (user_id, project_id, updated_at_ms DESC)
  WHERE archived_at_ms IS NULL;
