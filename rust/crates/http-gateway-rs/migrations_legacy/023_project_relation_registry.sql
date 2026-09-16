-- Unified project-to-project relationship registry for delete guards and future role-aware wiring. Author: kejiqing

CREATE TABLE IF NOT EXISTS project_relation (
  cluster_id TEXT NOT NULL,
  relation_type TEXT NOT NULL,
  from_proj_id BIGINT NOT NULL,
  to_proj_id BIGINT NOT NULL,
  relation_label TEXT,
  relation_meta_json JSONB NOT NULL DEFAULT '{}'::jsonb,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  PRIMARY KEY (cluster_id, relation_type, from_proj_id, to_proj_id),
  CHECK (relation_type IN ('router_delegate', 'master_apprentice', 'master_observation')),
  CHECK (from_proj_id <> to_proj_id)
);

CREATE INDEX IF NOT EXISTS idx_project_relation_to
  ON project_relation (cluster_id, to_proj_id, relation_type, from_proj_id);

CREATE INDEX IF NOT EXISTS idx_project_relation_from
  ON project_relation (cluster_id, from_proj_id, relation_type, to_proj_id);

INSERT INTO project_relation (
  cluster_id, relation_type, from_proj_id, to_proj_id,
  relation_label, relation_meta_json, created_at_ms, updated_at_ms
)
SELECT
  cluster_id,
  'router_delegate',
  initiator_proj_id,
  target_proj_id,
  label,
  jsonb_build_object(
    'enabled', enabled,
    'capabilityHint', capability_hint
  ),
  created_at_ms,
  updated_at_ms
FROM gateway_delegate_target
ON CONFLICT (cluster_id, relation_type, from_proj_id, to_proj_id) DO UPDATE SET
  relation_label = EXCLUDED.relation_label,
  relation_meta_json = EXCLUDED.relation_meta_json,
  updated_at_ms = EXCLUDED.updated_at_ms;

INSERT INTO project_relation (
  cluster_id, relation_type, from_proj_id, to_proj_id,
  relation_label, relation_meta_json, created_at_ms, updated_at_ms
)
SELECT
  cluster_id,
  'master_apprentice',
  master_proj_id,
  apprentice_proj_id,
  NULL,
  jsonb_build_object(
    'orphaned', orphaned,
    'gatewayBase', COALESCE(apprentice_gateway_base, '')
  ),
  created_at_ms,
  updated_at_ms
FROM project_master_link
WHERE orphaned = FALSE
ON CONFLICT (cluster_id, relation_type, from_proj_id, to_proj_id) DO UPDATE SET
  relation_meta_json = EXCLUDED.relation_meta_json,
  updated_at_ms = EXCLUDED.updated_at_ms;

INSERT INTO project_relation (
  cluster_id, relation_type, from_proj_id, to_proj_id,
  relation_label, relation_meta_json, created_at_ms, updated_at_ms
)
SELECT
  cluster_id,
  'master_observation',
  master_proj_id,
  observation_proj_id,
  NULL,
  jsonb_build_object(
    'orphaned', orphaned,
    'gatewayBase', COALESCE(apprentice_gateway_base, ''),
    'apprenticeProjId', apprentice_proj_id
  ),
  created_at_ms,
  updated_at_ms
FROM project_master_link
WHERE orphaned = FALSE
ON CONFLICT (cluster_id, relation_type, from_proj_id, to_proj_id) DO UPDATE SET
  relation_meta_json = EXCLUDED.relation_meta_json,
  updated_at_ms = EXCLUDED.updated_at_ms;
