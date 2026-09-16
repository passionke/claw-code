-- Project-bound model API keys + OpenAI-compat conversation/response maps.
-- Author: kejiqing

CREATE TABLE IF NOT EXISTS gateway_project_model_api_key (
  id TEXT PRIMARY KEY,
  cluster_id TEXT NOT NULL DEFAULT '',
  proj_id BIGINT NOT NULL,
  model_alias TEXT NOT NULL DEFAULT 'agent',
  name TEXT NOT NULL DEFAULT '',
  note TEXT NOT NULL DEFAULT '',
  token_hash TEXT NOT NULL,
  token_prefix TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT 'active',
  created_at_ms BIGINT NOT NULL,
  revoked_at_ms BIGINT,
  last_used_at_ms BIGINT
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_gateway_project_model_api_key_hash
  ON gateway_project_model_api_key (token_hash);

CREATE INDEX IF NOT EXISTS ix_gateway_project_model_api_key_proj
  ON gateway_project_model_api_key (cluster_id, proj_id);

CREATE TABLE IF NOT EXISTS gateway_openai_conversation (
  id TEXT PRIMARY KEY,
  cluster_id TEXT NOT NULL DEFAULT '',
  api_key_id TEXT NOT NULL,
  proj_id BIGINT NOT NULL,
  client_conversation_key TEXT NOT NULL,
  session_id TEXT NOT NULL,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS ux_gateway_openai_conversation_key
  ON gateway_openai_conversation (api_key_id, client_conversation_key);

CREATE INDEX IF NOT EXISTS ix_gateway_openai_conversation_session
  ON gateway_openai_conversation (proj_id, session_id);

CREATE TABLE IF NOT EXISTS gateway_openai_response (
  response_id TEXT PRIMARY KEY,
  cluster_id TEXT NOT NULL DEFAULT '',
  api_key_id TEXT NOT NULL,
  proj_id BIGINT NOT NULL,
  session_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  created_at_ms BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS ix_gateway_openai_response_turn
  ON gateway_openai_response (session_id, turn_id);
