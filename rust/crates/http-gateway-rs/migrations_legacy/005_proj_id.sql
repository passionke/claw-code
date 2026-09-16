-- Add proj_id column (mirror ds_id) for ds_id→proj_id rename. Author: kejiqing

ALTER TABLE gateway_sessions ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE gateway_sessions SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE gateway_sessions ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_gateway_sessions_session_proj ON gateway_sessions (session_id, proj_id);

ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE gateway_turns SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE gateway_turns ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_gateway_turns_session_proj ON gateway_turns (session_id, proj_id);
CREATE INDEX IF NOT EXISTS idx_gateway_turns_session_proj_status ON gateway_turns (session_id, proj_id, status, created_at_ms);

ALTER TABLE gateway_feedback ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE gateway_feedback SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE gateway_feedback ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_gateway_feedback_session_proj ON gateway_feedback (session_id, proj_id);

ALTER TABLE gateway_session_artifacts ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE gateway_session_artifacts SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE gateway_session_artifacts ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_gateway_session_artifacts_session_proj ON gateway_session_artifacts (session_id, proj_id, turn_id, relative_path);

ALTER TABLE cc_messages ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE cc_messages SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE cc_messages ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_cc_messages_session_proj ON cc_messages (session_id, proj_id, created_at_ms);

ALTER TABLE project_config ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE project_config SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE project_config ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_project_config_proj_id ON project_config (proj_id);

ALTER TABLE project_config_revision ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE project_config_revision SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE project_config_revision ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_project_config_revision_proj ON project_config_revision (proj_id, content_rev);

ALTER TABLE gateway_conversation_translate ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE gateway_conversation_translate SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE gateway_conversation_translate ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_gateway_conversation_translate_session_proj ON gateway_conversation_translate (session_id, proj_id);

ALTER TABLE project_entity_revision ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE project_entity_revision SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE project_entity_revision ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_project_entity_revision_proj ON project_entity_revision (proj_id, domain, entity_key, created_at_ms DESC);
