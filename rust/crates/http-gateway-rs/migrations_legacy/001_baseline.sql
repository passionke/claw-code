-- Baseline gateway session index tables (historical inline DDL). Author: kejiqing

CREATE TABLE IF NOT EXISTS gateway_sessions (
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    session_home TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (session_id, ds_id)
);

CREATE TABLE IF NOT EXISTS gateway_turns (
    turn_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    finished_at_ms BIGINT
);

CREATE INDEX IF NOT EXISTS idx_gateway_turns_session ON gateway_turns(session_id, ds_id);

CREATE TABLE IF NOT EXISTS gateway_feedback (
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    turn_id TEXT NOT NULL,
    feedback TEXT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (session_id, ds_id, turn_id)
);
