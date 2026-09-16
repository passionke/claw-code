-- cluster_id scoping for multi-gateway shared PG. Author: kejiqing
-- Backfill cluster_id applied at runtime from CLAW_CLUSTER_ID before NOT NULL enforcement.

ALTER TABLE gateway_sessions ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE gateway_feedback ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE gateway_conversation_translate ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE cc_messages ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE gateway_runtime_iterations ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE gateway_session_artifacts ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE project_config ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE project_config_revision ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE project_entity_revision ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE gateway_global_settings ADD COLUMN IF NOT EXISTS cluster_id TEXT;

CREATE INDEX IF NOT EXISTS idx_gateway_sessions_cluster ON gateway_sessions (cluster_id, proj_id);
CREATE INDEX IF NOT EXISTS idx_gateway_turns_cluster ON gateway_turns (cluster_id, proj_id);
CREATE INDEX IF NOT EXISTS idx_project_config_cluster ON project_config (cluster_id, proj_id);
