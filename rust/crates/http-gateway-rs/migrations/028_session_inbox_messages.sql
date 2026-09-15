-- Session inbox (gateway-held steer queue). Author: kejiqing
CREATE TABLE IF NOT EXISTS session_inbox_messages (
    cluster_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    source TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL,
    idempotency_key TEXT,
    created_at_ms BIGINT NOT NULL,
    consumed_at_ms BIGINT,
    consumed_turn_id TEXT,
    iteration INT,
    dropped_at_ms BIGINT,
    dropped_reason TEXT,
    PRIMARY KEY (cluster_id, message_id)
);

CREATE INDEX IF NOT EXISTS idx_session_inbox_queued
    ON session_inbox_messages (cluster_id, session_id, status, created_at_ms, message_id);

CREATE UNIQUE INDEX IF NOT EXISTS idx_session_inbox_idempotency
    ON session_inbox_messages (cluster_id, session_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
