-- Allow project_role=steerable for session inbox mid-turn steer. Author: kejiqing

ALTER TABLE project_config
  DROP CONSTRAINT IF EXISTS project_config_project_role_check;

ALTER TABLE project_config
  ADD CONSTRAINT project_config_project_role_check
  CHECK (project_role IN (
    'normal',
    'master',
    'observation',
    'router',
    'knowledge_base',
    'steerable'
  ));
