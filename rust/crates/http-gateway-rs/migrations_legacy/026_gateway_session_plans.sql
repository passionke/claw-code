-- First-class session plans (Cursor-style Plan mode SoT). Author: kejiqing

CREATE TABLE IF NOT EXISTS gateway_session_plans (
  plan_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  proj_id BIGINT NOT NULL,
  cluster_id TEXT NOT NULL DEFAULT '',
  title TEXT NOT NULL DEFAULT '',
  body_markdown TEXT NOT NULL,
  status TEXT NOT NULL,
  plan_turn_id TEXT NOT NULL,
  execute_turn_id TEXT,
  sealed_at_ms BIGINT,
  created_at_ms BIGINT NOT NULL,
  updated_at_ms BIGINT NOT NULL,
  created_by_prompt TEXT
);

CREATE INDEX IF NOT EXISTS idx_gateway_session_plans_session_proj
  ON gateway_session_plans (session_id, proj_id, created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_gateway_session_plans_plan_turn
  ON gateway_session_plans (plan_turn_id);

CREATE INDEX IF NOT EXISTS idx_gateway_session_plans_awaiting
  ON gateway_session_plans (session_id, proj_id)
  WHERE status = 'awaiting_confirm';
