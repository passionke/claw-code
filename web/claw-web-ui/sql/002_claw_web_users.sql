-- Claw Web users + scope conversations per user (L5: user_id = JWT sub). Author: kejiqing
--
-- Model:
--   user (claw_users)
--     └── project (see 003_claw_projects)
--           └── session (sessionId)
--                 └── tunnel (tunnel_id) → messages

CREATE TABLE IF NOT EXISTS claw_users (
  user_id TEXT PRIMARY KEY,
  tenant_id TEXT,
  display_name TEXT NOT NULL DEFAULT '',
  email TEXT,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  last_seen_at_ms BIGINT
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_claw_users_tenant_email
  ON claw_users (tenant_id, lower(email))
  WHERE email IS NOT NULL AND btrim(email) <> '';

CREATE INDEX IF NOT EXISTS idx_claw_users_tenant
  ON claw_users (tenant_id);

INSERT INTO claw_users (
  user_id, tenant_id, display_name, created_at_ms, updated_at_ms, last_seen_at_ms
)
VALUES (
  'dev-local',
  NULL,
  'Local Dev',
  (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT,
  (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT,
  (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT
)
ON CONFLICT (user_id) DO NOTHING;

-- --- migrate conversation tables from 001 (single-tenant) to per-user (idempotent) ---

ALTER TABLE claw_project_state ADD COLUMN IF NOT EXISTS user_id TEXT;
UPDATE claw_project_state SET user_id = 'dev-local' WHERE user_id IS NULL;

ALTER TABLE claw_sessions ADD COLUMN IF NOT EXISTS user_id TEXT;
UPDATE claw_sessions SET user_id = 'dev-local' WHERE user_id IS NULL;

ALTER TABLE claw_messages ADD COLUMN IF NOT EXISTS user_id TEXT;
UPDATE claw_messages SET user_id = 'dev-local' WHERE user_id IS NULL;

DO $user_scope$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.key_column_usage
    WHERE table_schema = 'public' AND table_name = 'claw_project_state'
      AND constraint_name = 'claw_project_state_pkey' AND column_name = 'user_id'
  ) THEN
    ALTER TABLE claw_project_state DROP CONSTRAINT IF EXISTS claw_project_state_pkey;
    ALTER TABLE claw_project_state DROP CONSTRAINT IF EXISTS claw_web_project_state_pkey;
    ALTER TABLE claw_project_state ALTER COLUMN user_id SET NOT NULL;
    ALTER TABLE claw_project_state ADD PRIMARY KEY (user_id, project_id);
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM information_schema.key_column_usage
    WHERE table_schema = 'public' AND table_name = 'claw_sessions'
      AND constraint_name = 'claw_sessions_pkey' AND column_name = 'user_id'
  ) THEN
    ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_messages_project_id_session_id_fkey;
    ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_web_messages_project_id_session_id_fkey;
    ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_messages_session_fkey;
    ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_web_messages_session_fkey;
    ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_messages_tunnel_fkey;
    ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_web_messages_tunnel_fkey;
    ALTER TABLE claw_tunnels DROP CONSTRAINT IF EXISTS claw_tunnels_session_fkey;
    ALTER TABLE claw_tunnels DROP CONSTRAINT IF EXISTS claw_web_tunnels_session_fkey;

    ALTER TABLE claw_sessions DROP CONSTRAINT IF EXISTS claw_sessions_pkey;
    ALTER TABLE claw_sessions DROP CONSTRAINT IF EXISTS claw_web_sessions_pkey;
    ALTER TABLE claw_sessions ALTER COLUMN user_id SET NOT NULL;
    ALTER TABLE claw_sessions ADD PRIMARY KEY (user_id, project_id, session_id);
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM information_schema.key_column_usage
    WHERE table_schema = 'public' AND table_name = 'claw_messages'
      AND constraint_name = 'claw_messages_pkey' AND column_name = 'user_id'
  ) THEN
    ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_messages_pkey;
    ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_web_messages_pkey;
    ALTER TABLE claw_messages ALTER COLUMN user_id SET NOT NULL;
    ALTER TABLE claw_messages ADD PRIMARY KEY (user_id, project_id, session_id, message_id);
  END IF;

  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'claw_messages_session_fkey' AND conrelid = 'claw_messages'::regclass
  ) THEN
    ALTER TABLE claw_messages ADD CONSTRAINT claw_messages_session_fkey
      FOREIGN KEY (user_id, project_id, session_id)
      REFERENCES claw_sessions (user_id, project_id, session_id) ON DELETE CASCADE;
  END IF;
END $user_scope$;

DROP INDEX IF EXISTS idx_claw_sessions_updated;
DROP INDEX IF EXISTS idx_claw_web_sessions_updated;
CREATE INDEX IF NOT EXISTS idx_claw_sessions_user_project_updated
  ON claw_sessions (user_id, project_id, updated_at_ms DESC);
