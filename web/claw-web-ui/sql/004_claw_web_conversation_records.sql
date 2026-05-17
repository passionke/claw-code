-- Conversation records: session list + tunnel (one user turn) + messages. Author: kejiqing
--
-- Full chain (after 002 users, 003 projects):
--   user → project → session → tunnel → message (user | assistant)

-- Per-user per-project UI state (active session in sidebar)
-- PK: (user_id, project_id)  [altered in 002]

-- claw_sessions: one chat thread (= gateway sessionId / AG-UI threadId)
-- PK: (user_id, project_id, session_id)

-- claw_tunnels: one user send (= runId scope at gateway)
-- PK: (user_id, project_id, session_id, tunnel_id)

CREATE TABLE IF NOT EXISTS claw_tunnels (
  user_id TEXT NOT NULL,
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  tunnel_id TEXT NOT NULL,
  run_id TEXT,
  status TEXT NOT NULL DEFAULT 'completed'
    CHECK (status IN ('pending', 'streaming', 'completed', 'failed')),
  user_preview TEXT NOT NULL DEFAULT '',
  error_preview TEXT,
  started_at_ms BIGINT NOT NULL,
  finished_at_ms BIGINT,
  PRIMARY KEY (user_id, project_id, session_id, tunnel_id),
  FOREIGN KEY (user_id, project_id, session_id)
    REFERENCES claw_sessions (user_id, project_id, session_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_claw_tunnels_session_started
  ON claw_tunnels (user_id, project_id, session_id, started_at_ms ASC);

-- claw_messages: each line shown in sidebar (user or assistant bubble)
-- PK: (user_id, project_id, session_id, message_id)
-- tunnel_id groups messages into one turn

ALTER TABLE claw_messages ADD COLUMN IF NOT EXISTS run_id TEXT;

ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_messages_tunnel_fkey;
ALTER TABLE claw_messages DROP CONSTRAINT IF EXISTS claw_web_messages_tunnel_fkey;
DO $tunnel_fk$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'claw_messages_tunnel_fkey' AND conrelid = 'claw_messages'::regclass
  ) THEN
    ALTER TABLE claw_messages ADD CONSTRAINT claw_messages_tunnel_fkey
      FOREIGN KEY (user_id, project_id, session_id, tunnel_id)
      REFERENCES claw_tunnels (user_id, project_id, session_id, tunnel_id) ON DELETE CASCADE;
  END IF;
END $tunnel_fk$;

-- Backfill tunnels from existing messages (one row per distinct tunnel_id)
INSERT INTO claw_tunnels (
  user_id, project_id, session_id, tunnel_id, status, user_preview, started_at_ms, finished_at_ms
)
SELECT
  m.user_id,
  m.project_id,
  m.session_id,
  m.tunnel_id,
  'completed',
  COALESCE(
    (SELECT LEFT(u.content, 240) FROM claw_messages u
     WHERE u.user_id = m.user_id AND u.project_id = m.project_id
       AND u.session_id = m.session_id AND u.tunnel_id = m.tunnel_id
       AND u.role = 'user' ORDER BY u.seq ASC LIMIT 1),
    ''
  ),
  MIN(m.created_at_ms),
  MAX(m.created_at_ms)
FROM claw_messages m
GROUP BY m.user_id, m.project_id, m.session_id, m.tunnel_id
ON CONFLICT (user_id, project_id, session_id, tunnel_id) DO NOTHING;
