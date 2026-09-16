-- Pool v1: session artifacts in PG, per-turn incremental cc_messages, enqueue gates. Author: kejiqing

ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS artifacts_ready BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS solve_task_json JSONB;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS solve_timing_jsonb JSONB;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS spill_json JSONB;

ALTER TABLE gateway_session_artifacts ADD COLUMN IF NOT EXISTS content TEXT;
ALTER TABLE gateway_session_artifacts ADD COLUMN IF NOT EXISTS content_json JSONB;

CREATE UNIQUE INDEX IF NOT EXISTS gateway_session_artifacts_session_ds_turn_path_key
    ON gateway_session_artifacts (session_id, ds_id, turn_id, relative_path);

CREATE INDEX IF NOT EXISTS idx_gateway_turns_session_status
    ON gateway_turns(session_id, ds_id, status, created_at_ms);
