-- Multi-gateway same clusterId: endpoint registry, turn owner, worker busy. Author: kejiqing

CREATE TABLE IF NOT EXISTS gateway_endpoint (
    cluster_id TEXT NOT NULL,
    gateway_id TEXT NOT NULL,
    gateway_base TEXT NOT NULL,
    hostname TEXT NOT NULL DEFAULT '',
    started_at_ms BIGINT NOT NULL DEFAULT 0,
    last_heartbeat_ms BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (cluster_id, gateway_id)
);

CREATE INDEX IF NOT EXISTS idx_gateway_endpoint_heartbeat
    ON gateway_endpoint (cluster_id, last_heartbeat_ms DESC);

ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS gateway_id TEXT;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS gateway_base TEXT;

CREATE INDEX IF NOT EXISTS idx_gateway_turns_gateway_id
    ON gateway_turns (cluster_id, gateway_id)
    WHERE gateway_id IS NOT NULL;

ALTER TABLE project_e2b_worker ADD COLUMN IF NOT EXISTS in_use_count INT NOT NULL DEFAULT 0;
ALTER TABLE project_e2b_worker ADD COLUMN IF NOT EXISTS in_use_until_ms BIGINT NOT NULL DEFAULT 0;
