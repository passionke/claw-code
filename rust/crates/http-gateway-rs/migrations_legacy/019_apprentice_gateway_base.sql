-- Optional apprentice gateway base for cross-gateway master pairing. Author: kejiqing
-- Empty string = local / this gateway (default).

ALTER TABLE project_master_link
  ADD COLUMN IF NOT EXISTS apprentice_gateway_base TEXT NOT NULL DEFAULT '';
