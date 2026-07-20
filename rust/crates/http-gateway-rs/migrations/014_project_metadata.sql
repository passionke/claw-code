-- Project metadata: human-readable code + description (sidecar, not in revision). Author: kejiqing

ALTER TABLE project_config ADD COLUMN IF NOT EXISTS project_code TEXT NOT NULL DEFAULT '';
ALTER TABLE project_config ADD COLUMN IF NOT EXISTS project_description TEXT NOT NULL DEFAULT '';

CREATE UNIQUE INDEX IF NOT EXISTS idx_project_config_code_unique
    ON project_config (cluster_id, project_code)
    WHERE project_code <> '';
