-- Observation id uniqueness scoped by apprentice gateway (cross-cluster). Author: kejiqing

DROP INDEX IF EXISTS idx_project_master_link_observation;

CREATE UNIQUE INDEX IF NOT EXISTS idx_project_master_link_observation
  ON project_master_link (cluster_id, apprentice_gateway_base, observation_proj_id);
