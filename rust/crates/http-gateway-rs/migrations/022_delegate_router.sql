-- Router role + delegate targets + delegate session links. Author: kejiqing

ALTER TABLE project_config
  DROP CONSTRAINT IF EXISTS project_config_project_role_check;

ALTER TABLE project_config
  ADD CONSTRAINT project_config_project_role_check
  CHECK (project_role IN ('normal', 'master', 'observation', 'router'));

CREATE TABLE IF NOT EXISTS gateway_delegate_target (
  cluster_id TEXT NOT NULL,
  initiator_proj_id BIGINT NOT NULL,
  target_proj_id BIGINT NOT NULL,
  enabled BOOLEAN NOT NULL DEFAULT TRUE,
  label TEXT,
  capability_hint TEXT,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  PRIMARY KEY (cluster_id, initiator_proj_id, target_proj_id)
);

CREATE INDEX IF NOT EXISTS idx_gateway_delegate_target_initiator
  ON gateway_delegate_target (cluster_id, initiator_proj_id);

CREATE TABLE IF NOT EXISTS gateway_delegate_session_link (
  cluster_id TEXT NOT NULL,
  root_session_id TEXT NOT NULL,
  parent_session_id TEXT NOT NULL,
  parent_proj_id BIGINT NOT NULL,
  delegate_proj_id BIGINT NOT NULL,
  delegate_session_id TEXT NOT NULL,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  PRIMARY KEY (parent_session_id, parent_proj_id, delegate_proj_id)
);

CREATE INDEX IF NOT EXISTS idx_gateway_delegate_session_link_root
  ON gateway_delegate_session_link (cluster_id, root_session_id);
