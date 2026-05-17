-- Project center storage: OSS (S3-compatible) or local gateway workspace. Author: kejiqing
--
-- Metadata in PG; blobs under oss://{bucket}/{oss_prefix} when storage_protocol = 'oss'.
-- Credentials only in env (CLAW_OSS_*), never in DB.
-- Gateway worker pool still uses CLAW_WORK_ROOT/ds_{ds_id}/ for solve; OSS is Web canonical store.

ALTER TABLE claw_projects ADD COLUMN IF NOT EXISTS storage_protocol TEXT NOT NULL DEFAULT 'local'
  CHECK (storage_protocol IN ('local', 'oss'));

ALTER TABLE claw_projects ADD COLUMN IF NOT EXISTS oss_bucket TEXT;
ALTER TABLE claw_projects ADD COLUMN IF NOT EXISTS oss_prefix TEXT NOT NULL DEFAULT '';
ALTER TABLE claw_projects ADD COLUMN IF NOT EXISTS oss_endpoint TEXT;
ALTER TABLE claw_projects ADD COLUMN IF NOT EXISTS oss_region TEXT;

UPDATE claw_projects
SET oss_prefix = 'claw/projects/_/' || project_id || '/'
WHERE oss_prefix = '' OR oss_prefix IS NULL;
