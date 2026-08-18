-- knowledge_base role, project kb sources, and generic scheduled job kind. Author: kejiqing

ALTER TABLE project_config
  ADD COLUMN IF NOT EXISTS kb_sources_json JSONB NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE project_config
  DROP CONSTRAINT IF EXISTS project_config_project_role_check;

ALTER TABLE project_config
  ADD CONSTRAINT project_config_project_role_check
  CHECK (project_role IN ('normal', 'master', 'observation', 'router', 'knowledge_base'));

ALTER TABLE gateway_scheduled_job
  ADD COLUMN IF NOT EXISTS job_kind TEXT NOT NULL DEFAULT 'master_digest';

ALTER TABLE gateway_scheduled_job
  DROP CONSTRAINT IF EXISTS gateway_scheduled_job_job_kind_check;

ALTER TABLE gateway_scheduled_job
  ADD CONSTRAINT gateway_scheduled_job_job_kind_check
  CHECK (job_kind IN ('master_digest', 'master_repair', 'kb_sync'));

UPDATE gateway_scheduled_job
SET job_kind = CASE
  WHEN schedule_kind = 'weekly' THEN 'master_repair'
  ELSE 'master_digest'
END
WHERE job_kind IS NULL OR job_kind = '';
