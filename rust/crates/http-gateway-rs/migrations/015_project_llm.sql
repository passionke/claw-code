-- Per-project LLM models + observe state (cluster_id scoped). Author: kejiqing

CREATE TABLE IF NOT EXISTS gateway_llm_project_model (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    model_id TEXT NOT NULL,
    name TEXT NOT NULL,
    base_model_url TEXT NOT NULL,
    model_name TEXT NOT NULL,
    current_rev TEXT NOT NULL DEFAULT '',
    api_key_ciphertext TEXT NOT NULL DEFAULT '',
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, proj_id, model_id)
);

CREATE TABLE IF NOT EXISTS gateway_llm_project_state (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    active_model_id TEXT NOT NULL DEFAULT '',
    active_model_rev TEXT NOT NULL DEFAULT '',
    active_applied_at_ms BIGINT,
    updated_at_ms BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (cluster_id, proj_id)
);

CREATE TABLE IF NOT EXISTS gateway_llm_project_revision (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    model_id TEXT NOT NULL,
    model_rev TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    name TEXT NOT NULL,
    base_model_url TEXT NOT NULL,
    model_name TEXT NOT NULL,
    supports_vision BOOLEAN NOT NULL DEFAULT FALSE,
    note TEXT,
    PRIMARY KEY (cluster_id, proj_id, model_id, model_rev)
);

CREATE INDEX IF NOT EXISTS idx_gateway_llm_project_revision_list
    ON gateway_llm_project_revision (cluster_id, proj_id, model_id, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS gateway_llm_project_observe (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    sandbox_id TEXT NOT NULL DEFAULT '',
    proxy_base_url TEXT NOT NULL DEFAULT '',
    live_base_url TEXT NOT NULL DEFAULT '',
    host TEXT NOT NULL DEFAULT '',
    proxy_port INT NOT NULL DEFAULT 8080,
    live_port INT NOT NULL DEFAULT 3000,
    updated_at_ms BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (cluster_id, proj_id)
);
