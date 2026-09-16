-- Complete cluster_id column coverage for shared PG (phase 1: nullable + indexes). Author: kejiqing
-- Phase 2 (separate migration): NOT NULL + PK (cluster_id, proj_id) on project_config, etc.

ALTER TABLE project_e2b_worker ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE worker_rotation_log ADD COLUMN IF NOT EXISTS cluster_id TEXT;
ALTER TABLE claw_pool ADD COLUMN IF NOT EXISTS cluster_id TEXT;

CREATE INDEX IF NOT EXISTS idx_project_e2b_worker_cluster ON project_e2b_worker (cluster_id, proj_id);
CREATE INDEX IF NOT EXISTS idx_worker_rotation_log_cluster ON worker_rotation_log (cluster_id, proj_id, at_ms DESC);
CREATE INDEX IF NOT EXISTS idx_claw_pool_cluster ON claw_pool (cluster_id);

CREATE INDEX IF NOT EXISTS idx_gateway_feedback_cluster ON gateway_feedback (cluster_id, proj_id);
CREATE INDEX IF NOT EXISTS idx_cc_messages_cluster ON cc_messages (cluster_id, proj_id);
CREATE INDEX IF NOT EXISTS idx_gateway_session_artifacts_cluster ON gateway_session_artifacts (cluster_id, proj_id);
CREATE INDEX IF NOT EXISTS idx_gateway_runtime_iterations_cluster ON gateway_runtime_iterations (cluster_id);
CREATE INDEX IF NOT EXISTS idx_gateway_conversation_translate_cluster ON gateway_conversation_translate (cluster_id, proj_id);
CREATE INDEX IF NOT EXISTS idx_project_config_revision_cluster ON project_config_revision (cluster_id, proj_id);
CREATE INDEX IF NOT EXISTS idx_project_entity_revision_cluster ON project_entity_revision (cluster_id, proj_id);
