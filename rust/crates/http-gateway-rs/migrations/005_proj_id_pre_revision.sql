-- proj_id columns required before project_config_revision seed INSERT. Author: kejiqing

ALTER TABLE project_config ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE project_config SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE project_config ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_project_config_proj_id ON project_config (proj_id);

ALTER TABLE project_config_revision ADD COLUMN IF NOT EXISTS proj_id BIGINT;
UPDATE project_config_revision SET proj_id = ds_id WHERE proj_id IS NULL;
ALTER TABLE project_config_revision ALTER COLUMN proj_id SET NOT NULL;
CREATE INDEX IF NOT EXISTS idx_project_config_revision_proj ON project_config_revision (proj_id, content_rev);
