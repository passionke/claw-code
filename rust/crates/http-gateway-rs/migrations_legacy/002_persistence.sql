-- DB SoT persistence: projects, messages, async tasks, turn metadata. Author: kejiqing

CREATE TABLE IF NOT EXISTS gateway_projects (
    ds_id BIGINT PRIMARY KEY,
    project_name TEXT NOT NULL,
    workspace_rel TEXT NOT NULL,
    description TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS gateway_project_git (
    ds_id BIGINT PRIMARY KEY REFERENCES gateway_projects(ds_id) ON DELETE CASCADE,
    remote_url TEXT,
    branch TEXT,
    last_synced_commit TEXT,
    last_synced_at_ms BIGINT
);

ALTER TABLE gateway_sessions ADD COLUMN IF NOT EXISTS title TEXT;
ALTER TABLE gateway_sessions ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'active';

CREATE TABLE IF NOT EXISTS gateway_async_tasks (
    task_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    active_turn_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    started_at_ms BIGINT,
    finished_at_ms BIGINT,
    current_task_desc TEXT,
    progress_updated_at_ms BIGINT,
    has_report BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_gateway_async_tasks_session ON gateway_async_tasks(session_id, ds_id);

ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS user_prompt TEXT;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS claw_exit_code INT;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS report_message TEXT;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS output_json JSONB;
ALTER TABLE gateway_turns ADD COLUMN IF NOT EXISTS has_report BOOLEAN NOT NULL DEFAULT FALSE;

CREATE TABLE IF NOT EXISTS gateway_runtime_iterations (
    iteration_id UUID PRIMARY KEY,
    turn_id TEXT NOT NULL REFERENCES gateway_turns(turn_id) ON DELETE CASCADE,
    iteration_index INT NOT NULL,
    started_at_ms BIGINT NOT NULL,
    finished_at_ms BIGINT,
    UNIQUE (turn_id, iteration_index)
);

CREATE TABLE IF NOT EXISTS cc_messages (
    message_id BIGSERIAL PRIMARY KEY,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    turn_id TEXT NOT NULL REFERENCES gateway_turns(turn_id) ON DELETE CASCADE,
    iteration_id UUID REFERENCES gateway_runtime_iterations(iteration_id) ON DELETE SET NULL,
    seq INT NOT NULL,
    role TEXT NOT NULL,
    blocks JSONB NOT NULL,
    usage JSONB,
    created_at_ms BIGINT NOT NULL,
    UNIQUE (turn_id, seq)
);

CREATE INDEX IF NOT EXISTS idx_cc_messages_session ON cc_messages(session_id, ds_id, created_at_ms);

CREATE TABLE IF NOT EXISTS gateway_turn_runtime_config (
    turn_id TEXT PRIMARY KEY REFERENCES gateway_turns(turn_id) ON DELETE CASCADE,
    system_prompt_sections JSONB,
    system_prompt_hash TEXT,
    allowed_tools JSONB,
    max_iterations INT,
    model TEXT,
    extra_session JSONB,
    mcp_servers_snapshot JSONB
);

CREATE TABLE IF NOT EXISTS gateway_turn_container_runs (
    turn_id TEXT PRIMARY KEY REFERENCES gateway_turns(turn_id) ON DELETE CASCADE,
    pool_slot_index INT,
    worker_container_id TEXT,
    worker_image TEXT,
    session_mount_path TEXT,
    started_at_ms BIGINT NOT NULL,
    finished_at_ms BIGINT,
    duration_ms BIGINT
);

CREATE TABLE IF NOT EXISTS gateway_model_usage (
    usage_id BIGSERIAL PRIMARY KEY,
    turn_id TEXT NOT NULL REFERENCES gateway_turns(turn_id) ON DELETE CASCADE,
    provider TEXT,
    model TEXT NOT NULL,
    input_tokens INT NOT NULL DEFAULT 0,
    output_tokens INT NOT NULL DEFAULT 0,
    cache_creation_input_tokens INT NOT NULL DEFAULT 0,
    cache_read_input_tokens INT NOT NULL DEFAULT 0,
    latency_ms BIGINT,
    source TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS gateway_session_artifacts (
    artifact_id UUID PRIMARY KEY,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    turn_id TEXT,
    kind TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    storage_uri TEXT,
    sha256 TEXT,
    size_bytes BIGINT,
    created_at_ms BIGINT NOT NULL
);
