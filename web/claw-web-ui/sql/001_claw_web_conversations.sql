-- Claw Web conversations (base; extended by 002 user_id, 004 tunnels). Author: kejiqing
-- Chain: user → project → session → tunnel (one turn) → message (user|assistant)

CREATE TABLE IF NOT EXISTS claw_project_state (
  project_id TEXT PRIMARY KEY,
  active_session_id TEXT,
  updated_at_ms BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS claw_sessions (
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '新对话',
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  PRIMARY KEY (project_id, session_id)
);

CREATE TABLE IF NOT EXISTS claw_messages (
  project_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  tunnel_id TEXT NOT NULL,
  message_id TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
  content TEXT NOT NULL,
  seq INTEGER NOT NULL,
  created_at_ms BIGINT NOT NULL,
  PRIMARY KEY (project_id, session_id, message_id),
  FOREIGN KEY (project_id, session_id)
    REFERENCES claw_sessions (project_id, session_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_claw_sessions_updated
  ON claw_sessions (project_id, updated_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_claw_messages_seq
  ON claw_messages (project_id, session_id, seq);
