-- Claw Web projects (workspace / dsId) + user membership. Author: kejiqing
--
-- Model:
--   user ──< user_projects >── project (ds_id)
--     └── sessions / messages (unchanged keys: user_id + project_id)

CREATE TABLE IF NOT EXISTS claw_projects (
  project_id TEXT PRIMARY KEY,
  ds_id INTEGER NOT NULL CHECK (ds_id >= 1),
  tenant_id TEXT,
  title TEXT NOT NULL DEFAULT '',
  description TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'archived')),
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_claw_projects_tenant_ds
  ON claw_projects (COALESCE(tenant_id, ''), ds_id);

CREATE INDEX IF NOT EXISTS idx_claw_projects_ds
  ON claw_projects (ds_id);

CREATE TABLE IF NOT EXISTS claw_user_projects (
  user_id TEXT NOT NULL REFERENCES claw_users (user_id) ON DELETE CASCADE,
  project_id TEXT NOT NULL REFERENCES claw_projects (project_id) ON DELETE CASCADE,
  role TEXT NOT NULL DEFAULT 'owner' CHECK (role IN ('owner', 'member', 'viewer')),
  joined_at_ms BIGINT NOT NULL,
  PRIMARY KEY (user_id, project_id)
);

CREATE INDEX IF NOT EXISTS idx_claw_user_projects_project
  ON claw_user_projects (project_id);

-- Backfill from existing conversation rows
INSERT INTO claw_projects (project_id, ds_id, tenant_id, title, created_at_ms, updated_at_ms)
SELECT DISTINCT
  pid,
  CASE WHEN pid ~ '^[0-9]+$' THEN pid::INTEGER ELSE 1 END,
  NULL,
  'Workspace ds ' || pid,
  (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT,
  (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT
FROM (
  SELECT project_id AS pid FROM claw_sessions
  UNION
  SELECT project_id FROM claw_project_state
) s
ON CONFLICT (project_id) DO NOTHING;

INSERT INTO claw_projects (project_id, ds_id, tenant_id, title, created_at_ms, updated_at_ms)
VALUES ('1', 1, NULL, 'Workspace ds 1', (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT, (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT)
ON CONFLICT (project_id) DO NOTHING;

INSERT INTO claw_user_projects (user_id, project_id, role, joined_at_ms)
SELECT DISTINCT s.user_id, s.project_id, 'owner', (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT
FROM claw_sessions s
INNER JOIN claw_projects p ON p.project_id = s.project_id
ON CONFLICT (user_id, project_id) DO NOTHING;

INSERT INTO claw_user_projects (user_id, project_id, role, joined_at_ms)
VALUES ('dev-local', '1', 'owner', (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT)
ON CONFLICT (user_id, project_id) DO NOTHING;

ALTER TABLE claw_project_state DROP CONSTRAINT IF EXISTS claw_project_state_project_fkey;
ALTER TABLE claw_project_state DROP CONSTRAINT IF EXISTS claw_web_project_state_project_fkey;
ALTER TABLE claw_sessions DROP CONSTRAINT IF EXISTS claw_sessions_project_fkey;
ALTER TABLE claw_sessions DROP CONSTRAINT IF EXISTS claw_web_sessions_project_fkey;
ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_messages_project_fkey;
ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_web_messages_project_fkey;

DO $project_fk$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'claw_project_state_project_fkey'
      AND conrelid = 'claw_project_state'::regclass
  ) THEN
    ALTER TABLE claw_project_state
      ADD CONSTRAINT claw_project_state_project_fkey
      FOREIGN KEY (project_id) REFERENCES claw_projects (project_id) ON DELETE CASCADE;
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'claw_sessions_project_fkey' AND conrelid = 'claw_sessions'::regclass
  ) THEN
    ALTER TABLE claw_sessions
      ADD CONSTRAINT claw_sessions_project_fkey
      FOREIGN KEY (project_id) REFERENCES claw_projects (project_id) ON DELETE CASCADE;
  END IF;
END $project_fk$;
