-- Squashed baseline: final schema after legacy replay-all migrator (001–030 + phase2/3).
-- Empty DBs apply this once; existing DBs are stamped as version 1 without re-running.
-- Author: kejiqing

CREATE TABLE gateway_sessions (
    cluster_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    proj_id BIGINT NOT NULL,
    session_home TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    client_origin TEXT,
    PRIMARY KEY (cluster_id, session_id, ds_id)
);

CREATE INDEX idx_gateway_sessions_session_proj ON gateway_sessions (session_id, proj_id);
CREATE INDEX idx_gateway_sessions_cluster ON gateway_sessions (cluster_id, proj_id);

CREATE TABLE gateway_turns (
    cluster_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    proj_id BIGINT NOT NULL,
    status TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    finished_at_ms BIGINT,
    user_prompt TEXT,
    report_message TEXT,
    output_json JSONB,
    claw_exit_code INT,
    pool_id TEXT,
    worker_name TEXT,
    worker_exec_user TEXT,
    client_origin TEXT,
    entry_params_json JSONB,
    artifacts_ready BOOLEAN NOT NULL DEFAULT FALSE,
    solve_task_json JSONB,
    solve_timing_jsonb JSONB,
    spill_json JSONB,
    gateway_id TEXT,
    gateway_base TEXT,
    PRIMARY KEY (cluster_id, turn_id)
);

CREATE INDEX idx_gateway_turns_session ON gateway_turns (session_id, ds_id);
CREATE INDEX idx_gateway_turns_session_proj ON gateway_turns (session_id, proj_id);
CREATE INDEX idx_gateway_turns_session_proj_status ON gateway_turns (session_id, proj_id, status, created_at_ms);
CREATE INDEX idx_gateway_turns_pool_id ON gateway_turns (pool_id);
CREATE INDEX idx_gateway_turns_cluster ON gateway_turns (cluster_id, proj_id);
CREATE INDEX idx_gateway_turns_gateway_id ON gateway_turns (cluster_id, gateway_id) WHERE gateway_id IS NOT NULL;

CREATE TABLE gateway_feedback (
    cluster_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    proj_id BIGINT NOT NULL,
    turn_id TEXT NOT NULL,
    feedback TEXT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, session_id, ds_id, turn_id)
);

CREATE INDEX idx_gateway_feedback_session_proj ON gateway_feedback (session_id, proj_id);
CREATE INDEX idx_gateway_feedback_cluster ON gateway_feedback (cluster_id, proj_id);

CREATE TABLE gateway_conversation_translate (
    cluster_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    proj_id BIGINT NOT NULL,
    source_fingerprint TEXT NOT NULL,
    turns_json JSONB NOT NULL,
    markdown TEXT NOT NULL,
    target_language TEXT NOT NULL DEFAULT 'zh-CN',
    model_id TEXT,
    status TEXT NOT NULL DEFAULT 'ready',
    error_text TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, session_id, ds_id)
);

CREATE INDEX idx_gateway_conversation_translate_session_proj
    ON gateway_conversation_translate (session_id, proj_id);
CREATE INDEX idx_gateway_conversation_translate_cluster
    ON gateway_conversation_translate (cluster_id, proj_id);

CREATE TABLE cc_messages (
    cluster_id TEXT NOT NULL,
    message_id BIGSERIAL NOT NULL,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    proj_id BIGINT NOT NULL,
    turn_id TEXT NOT NULL,
    iteration_id UUID,
    seq INT NOT NULL,
    role TEXT NOT NULL,
    blocks JSONB NOT NULL,
    usage JSONB,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, message_id),
    CONSTRAINT cc_messages_turn_id_seq_key UNIQUE (cluster_id, turn_id, seq),
    CONSTRAINT cc_messages_turn_id_fkey
        FOREIGN KEY (cluster_id, turn_id)
        REFERENCES gateway_turns (cluster_id, turn_id) ON DELETE CASCADE
);

CREATE INDEX idx_cc_messages_session ON cc_messages (session_id, ds_id, created_at_ms);
CREATE INDEX idx_cc_messages_session_proj ON cc_messages (session_id, proj_id, created_at_ms);
CREATE INDEX idx_cc_messages_cluster ON cc_messages (cluster_id, proj_id);

CREATE TABLE gateway_runtime_iterations (
    cluster_id TEXT NOT NULL,
    iteration_id UUID NOT NULL,
    turn_id TEXT NOT NULL,
    iteration_index INT NOT NULL,
    started_at_ms BIGINT NOT NULL,
    finished_at_ms BIGINT,
    PRIMARY KEY (cluster_id, iteration_id),
    CONSTRAINT gateway_runtime_iterations_turn_id_iteration_index_key
        UNIQUE (cluster_id, turn_id, iteration_index),
    CONSTRAINT gateway_runtime_iterations_turn_id_fkey
        FOREIGN KEY (cluster_id, turn_id)
        REFERENCES gateway_turns (cluster_id, turn_id) ON DELETE CASCADE
);

CREATE INDEX idx_gateway_runtime_iterations_cluster ON gateway_runtime_iterations (cluster_id);

CREATE TABLE gateway_model_usage (
    usage_id BIGSERIAL PRIMARY KEY,
    turn_id TEXT NOT NULL,
    provider TEXT,
    model TEXT NOT NULL,
    input_tokens INT NOT NULL DEFAULT 0,
    output_tokens INT NOT NULL DEFAULT 0,
    cache_creation_input_tokens INT NOT NULL DEFAULT 0,
    cache_read_input_tokens INT NOT NULL DEFAULT 0,
    latency_ms BIGINT,
    source TEXT NOT NULL
);

CREATE INDEX idx_gateway_model_usage_turn ON gateway_model_usage (turn_id);

CREATE TABLE gateway_session_artifacts (
    cluster_id TEXT NOT NULL,
    artifact_id UUID NOT NULL,
    session_id TEXT NOT NULL,
    ds_id BIGINT NOT NULL,
    proj_id BIGINT NOT NULL,
    turn_id TEXT,
    kind TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    storage_uri TEXT,
    sha256 TEXT,
    size_bytes BIGINT,
    content TEXT,
    content_json JSONB,
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, artifact_id),
    CONSTRAINT gateway_session_artifacts_session_ds_turn_path_key
        UNIQUE (cluster_id, session_id, ds_id, turn_id, relative_path)
);

CREATE INDEX idx_gateway_session_artifacts_session_proj
    ON gateway_session_artifacts (session_id, proj_id, turn_id, relative_path);
CREATE INDEX idx_gateway_session_artifacts_cluster
    ON gateway_session_artifacts (cluster_id, proj_id);

CREATE TABLE claw_pool (
    cluster_id TEXT NOT NULL,
    pool_id TEXT NOT NULL,
    registration_time_ms BIGINT NOT NULL,
    slots_max INT NOT NULL,
    slots_min INT NOT NULL,
    advertise_ip TEXT NOT NULL,
    sse_port INT NOT NULL,
    gateway_base TEXT NOT NULL DEFAULT '',
    last_heartbeat_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, pool_id)
);

CREATE INDEX idx_claw_pool_cluster ON claw_pool (cluster_id);

CREATE TABLE project_config (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    ds_id BIGINT NOT NULL,
    content_rev TEXT NOT NULL DEFAULT '',
    updated_at_ms BIGINT NOT NULL,
    rules_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    mcp_servers_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    skills_sources_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    allowed_tools_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    claude_md TEXT,
    skills_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    git_sync_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    stable_content_rev TEXT,
    draft_open BOOLEAN NOT NULL DEFAULT false,
    solve_preflight_json JSONB NOT NULL DEFAULT '{"kind":"none"}'::jsonb,
    solve_orchestration_json JSONB NOT NULL DEFAULT '{"kind":"single_turn"}'::jsonb,
    language_pipeline_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    extra_session_fields_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    prompt_limits_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    worker_profile_json JSONB NOT NULL DEFAULT '{"mode":"strict"}'::jsonb,
    worker_env_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    project_code TEXT NOT NULL DEFAULT '',
    project_description TEXT NOT NULL DEFAULT '',
    max_iterations INT,
    project_role TEXT NOT NULL DEFAULT 'normal',
    kb_sources_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    PRIMARY KEY (cluster_id, proj_id),
    CONSTRAINT project_config_project_role_check
        CHECK (project_role IN (
            'normal', 'master', 'observation', 'router', 'knowledge_base', 'steerable'
        ))
);

CREATE INDEX idx_project_config_proj_id ON project_config (proj_id);
CREATE INDEX idx_project_config_cluster ON project_config (cluster_id, proj_id);
CREATE UNIQUE INDEX idx_project_config_code_unique
    ON project_config (cluster_id, project_code)
    WHERE project_code <> '';

CREATE TABLE project_config_revision (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    ds_id BIGINT NOT NULL,
    content_rev TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    rules_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    mcp_servers_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    skills_sources_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    skills_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    allowed_tools_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    claude_md TEXT,
    note TEXT,
    PRIMARY KEY (cluster_id, proj_id, content_rev)
);

CREATE INDEX idx_project_config_revision_proj ON project_config_revision (proj_id, content_rev);
CREATE INDEX idx_project_config_revision_cluster ON project_config_revision (cluster_id, proj_id);

CREATE TABLE project_entity_revision (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    ds_id BIGINT NOT NULL,
    domain TEXT NOT NULL,
    entity_key TEXT NOT NULL,
    entity_rev TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    note TEXT,
    body JSONB NOT NULL,
    PRIMARY KEY (cluster_id, proj_id, domain, entity_key, entity_rev)
);

CREATE INDEX idx_project_entity_revision_list
    ON project_entity_revision (ds_id, domain, entity_key, created_at_ms DESC);
CREATE INDEX idx_project_entity_revision_proj
    ON project_entity_revision (proj_id, domain, entity_key, created_at_ms DESC);
CREATE INDEX idx_project_entity_revision_cluster
    ON project_entity_revision (cluster_id, proj_id);

CREATE TABLE gateway_global_settings (
    cluster_id TEXT PRIMARY KEY,
    settings_json JSONB NOT NULL DEFAULT '{"gitPats":[]}'::jsonb,
    git_pat_tokens_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at_ms BIGINT NOT NULL DEFAULT 0,
    system_prompt_default TEXT NOT NULL DEFAULT '',
    system_prompt_version TEXT NOT NULL DEFAULT 'v1',
    llm_base_model_url TEXT NOT NULL DEFAULT '',
    llm_model_name TEXT NOT NULL DEFAULT '',
    llm_model_api_key TEXT NOT NULL DEFAULT '',
    llm_model_updated_at_ms BIGINT NOT NULL DEFAULT 0,
    llm_model_applied_at_ms BIGINT,
    llm_models_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    llm_model_api_keys_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    active_llm_model_id TEXT NOT NULL DEFAULT '',
    active_llm_applied_at_ms BIGINT,
    active_llm_model_rev TEXT NOT NULL DEFAULT ''
);

CREATE TABLE gateway_llm_model_revision (
    model_id TEXT NOT NULL,
    model_rev TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    name TEXT NOT NULL,
    base_model_url TEXT NOT NULL,
    model_name TEXT NOT NULL,
    note TEXT,
    PRIMARY KEY (model_id, model_rev)
);

CREATE INDEX idx_gateway_llm_model_revision_list
    ON gateway_llm_model_revision (model_id, created_at_ms DESC);

CREATE TABLE gateway_llm_cluster_model (
    cluster_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    name TEXT NOT NULL,
    base_model_url TEXT NOT NULL,
    model_name TEXT NOT NULL,
    current_rev TEXT NOT NULL DEFAULT '',
    api_key_ciphertext TEXT NOT NULL DEFAULT '',
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, model_id)
);

CREATE TABLE gateway_llm_cluster_state (
    cluster_id TEXT PRIMARY KEY,
    active_model_id TEXT NOT NULL DEFAULT '',
    active_model_rev TEXT NOT NULL DEFAULT '',
    active_applied_at_ms BIGINT,
    updated_at_ms BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE gateway_llm_cluster_revision (
    cluster_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    model_rev TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    name TEXT NOT NULL,
    base_model_url TEXT NOT NULL,
    model_name TEXT NOT NULL,
    note TEXT,
    supports_vision BOOLEAN NOT NULL DEFAULT FALSE,
    supports_video BOOLEAN NOT NULL DEFAULT FALSE,
    supports_audio BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (cluster_id, model_id, model_rev)
);

CREATE INDEX idx_gateway_llm_cluster_revision_list
    ON gateway_llm_cluster_revision (cluster_id, model_id, created_at_ms DESC);

CREATE TABLE project_e2b_worker (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    slot_index INT NOT NULL DEFAULT 0,
    sandbox_id TEXT NOT NULL,
    worker_id TEXT NOT NULL,
    template_id TEXT NOT NULL,
    handle_json JSONB NOT NULL,
    updated_at_ms BIGINT NOT NULL DEFAULT 0,
    in_use_count INT NOT NULL DEFAULT 0,
    in_use_until_ms BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (cluster_id, proj_id, slot_index)
);

CREATE INDEX idx_project_e2b_worker_sandbox_id ON project_e2b_worker (sandbox_id);
CREATE INDEX idx_project_e2b_worker_cluster ON project_e2b_worker (cluster_id, proj_id);

CREATE TABLE worker_rotation_log (
    cluster_id TEXT NOT NULL,
    id BIGSERIAL NOT NULL,
    proj_id BIGINT NOT NULL,
    event TEXT NOT NULL,
    sandbox_id TEXT,
    worker_id TEXT,
    template_id TEXT,
    reason TEXT,
    at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, id)
);

CREATE INDEX idx_worker_rotation_log_proj ON worker_rotation_log (proj_id, at_ms DESC);
CREATE INDEX idx_worker_rotation_log_cluster ON worker_rotation_log (cluster_id, proj_id, at_ms DESC);

CREATE TABLE preflight_plugin (
    plugin_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    spi_version TEXT NOT NULL DEFAULT '1',
    default_impl JSONB,
    config_schema JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at_ms BIGINT NOT NULL DEFAULT 0
);

INSERT INTO preflight_plugin (plugin_id, display_name, spi_version, default_impl, config_schema, updated_at_ms)
VALUES
    (
        'turn_language',
        'Turn language inference',
        '1',
        '{"type":"builtin","handler":"turn_language"}'::jsonb,
        '{}'::jsonb,
        (EXTRACT(EPOCH FROM NOW()) * 1000)::bigint
    ),
    (
        'sqlbot_mcp_start',
        'SQLBot MCP start (session first turn)',
        '1',
        '{"type":"builtin","handler":"sqlbot_mcp_start"}'::jsonb,
        '{}'::jsonb,
        (EXTRACT(EPOCH FROM NOW()) * 1000)::bigint
    )
ON CONFLICT (plugin_id) DO NOTHING;

CREATE TABLE gateway_endpoint (
    cluster_id TEXT NOT NULL,
    gateway_id TEXT NOT NULL,
    gateway_base TEXT NOT NULL,
    hostname TEXT NOT NULL DEFAULT '',
    started_at_ms BIGINT NOT NULL DEFAULT 0,
    last_heartbeat_ms BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (cluster_id, gateway_id)
);

CREATE INDEX idx_gateway_endpoint_heartbeat
    ON gateway_endpoint (cluster_id, last_heartbeat_ms DESC);

CREATE TABLE gateway_llm_project_model (
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

CREATE TABLE gateway_llm_project_state (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    active_model_id TEXT NOT NULL DEFAULT '',
    active_model_rev TEXT NOT NULL DEFAULT '',
    active_applied_at_ms BIGINT,
    updated_at_ms BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (cluster_id, proj_id)
);

CREATE TABLE gateway_llm_project_revision (
    cluster_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    model_id TEXT NOT NULL,
    model_rev TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    name TEXT NOT NULL,
    base_model_url TEXT NOT NULL,
    model_name TEXT NOT NULL,
    supports_vision BOOLEAN NOT NULL DEFAULT FALSE,
    supports_video BOOLEAN NOT NULL DEFAULT FALSE,
    supports_audio BOOLEAN NOT NULL DEFAULT FALSE,
    note TEXT,
    PRIMARY KEY (cluster_id, proj_id, model_id, model_rev)
);

CREATE INDEX idx_gateway_llm_project_revision_list
    ON gateway_llm_project_revision (cluster_id, proj_id, model_id, created_at_ms DESC);

CREATE TABLE gateway_llm_project_observe (
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

CREATE TABLE project_master_link (
    cluster_id TEXT NOT NULL,
    master_proj_id BIGINT NOT NULL,
    apprentice_proj_id BIGINT NOT NULL,
    observation_proj_id BIGINT NOT NULL,
    orphaned BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    apprentice_gateway_base TEXT NOT NULL DEFAULT '',
    apprentice_mcp_token TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (cluster_id, master_proj_id, apprentice_proj_id)
);

CREATE UNIQUE INDEX idx_project_master_link_observation
    ON project_master_link (cluster_id, apprentice_gateway_base, observation_proj_id);

CREATE TABLE master_repair_run (
    cluster_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    master_proj_id BIGINT NOT NULL,
    apprentice_proj_id BIGINT NOT NULL,
    observation_proj_id BIGINT NOT NULL,
    master_session_id TEXT,
    master_turn_id TEXT,
    status TEXT NOT NULL DEFAULT 'opened',
    inventory_json JSONB NOT NULL DEFAULT '{"items":[]}'::jsonb,
    baseline_apprentice_content_rev TEXT,
    observation_content_rev_before TEXT,
    observation_content_rev_after TEXT,
    replay_session_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
    analysis_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    promote_status TEXT NOT NULL DEFAULT 'none',
    apprentice_draft_note TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, run_id)
);

CREATE INDEX idx_master_repair_run_master
    ON master_repair_run (cluster_id, master_proj_id, created_at_ms DESC);

CREATE TABLE gateway_scheduled_job (
    cluster_id TEXT NOT NULL,
    job_id TEXT NOT NULL,
    master_proj_id BIGINT NOT NULL,
    schedule_kind TEXT NOT NULL DEFAULT 'daily',
    run_at_hhmm TEXT NOT NULL DEFAULT '02:00',
    weekday INT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    prompt_template TEXT NOT NULL DEFAULT '',
    last_run_at_ms BIGINT,
    last_task_id TEXT,
    last_error TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    job_kind TEXT NOT NULL DEFAULT 'master_digest',
    PRIMARY KEY (cluster_id, job_id),
    CONSTRAINT gateway_scheduled_job_job_kind_check
        CHECK (job_kind IN ('master_digest', 'master_repair', 'kb_sync'))
);

CREATE INDEX idx_gateway_scheduled_job_master
    ON gateway_scheduled_job (cluster_id, master_proj_id);

CREATE TABLE gateway_delegate_target (
    cluster_id TEXT NOT NULL,
    initiator_proj_id BIGINT NOT NULL,
    target_proj_id BIGINT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    label TEXT,
    capability_hint TEXT,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, initiator_proj_id, target_proj_id)
);

CREATE INDEX idx_gateway_delegate_target_initiator
    ON gateway_delegate_target (cluster_id, initiator_proj_id);

CREATE TABLE gateway_delegate_session_link (
    cluster_id TEXT NOT NULL,
    root_session_id TEXT NOT NULL,
    parent_session_id TEXT NOT NULL,
    parent_proj_id BIGINT NOT NULL,
    delegate_proj_id BIGINT NOT NULL,
    delegate_session_id TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (parent_session_id, parent_proj_id, delegate_proj_id)
);

CREATE INDEX idx_gateway_delegate_session_link_root
    ON gateway_delegate_session_link (cluster_id, root_session_id);

CREATE TABLE project_relation (
    cluster_id TEXT NOT NULL,
    relation_type TEXT NOT NULL,
    from_proj_id BIGINT NOT NULL,
    to_proj_id BIGINT NOT NULL,
    relation_label TEXT,
    relation_meta_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, relation_type, from_proj_id, to_proj_id),
    CHECK (relation_type IN ('router_delegate', 'master_apprentice', 'master_observation')),
    CHECK (from_proj_id <> to_proj_id)
);

CREATE INDEX idx_project_relation_to
    ON project_relation (cluster_id, to_proj_id, relation_type, from_proj_id);
CREATE INDEX idx_project_relation_from
    ON project_relation (cluster_id, from_proj_id, relation_type, to_proj_id);

CREATE TABLE gateway_project_model_api_key (
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

CREATE UNIQUE INDEX ux_gateway_project_model_api_key_hash
    ON gateway_project_model_api_key (token_hash);
CREATE INDEX ix_gateway_project_model_api_key_proj
    ON gateway_project_model_api_key (cluster_id, proj_id);

CREATE TABLE gateway_openai_conversation (
    id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL DEFAULT '',
    api_key_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    client_conversation_key TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL
);

CREATE UNIQUE INDEX ux_gateway_openai_conversation_key
    ON gateway_openai_conversation (api_key_id, client_conversation_key);
CREATE INDEX ix_gateway_openai_conversation_session
    ON gateway_openai_conversation (proj_id, session_id);

CREATE TABLE gateway_openai_response (
    response_id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL DEFAULT '',
    api_key_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    session_id TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL
);

CREATE INDEX ix_gateway_openai_response_turn
    ON gateway_openai_response (session_id, turn_id);

CREATE TABLE gateway_session_plans (
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

CREATE INDEX idx_gateway_session_plans_session_proj
    ON gateway_session_plans (session_id, proj_id, created_at_ms DESC);
CREATE INDEX idx_gateway_session_plans_plan_turn
    ON gateway_session_plans (plan_turn_id);
CREATE INDEX idx_gateway_session_plans_awaiting
    ON gateway_session_plans (session_id, proj_id)
    WHERE status = 'awaiting_confirm';

CREATE TABLE gateway_admin_accounts (
    account_id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL,
    username TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    system_role TEXT NOT NULL DEFAULT 'none',
    disabled BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_ms BIGINT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    CONSTRAINT chk_gateway_admin_accounts_system_role
        CHECK (system_role IN ('system_admin', 'none'))
);

CREATE UNIQUE INDEX ux_gateway_admin_accounts_cluster_username
    ON gateway_admin_accounts (cluster_id, username);
CREATE INDEX ix_gateway_admin_accounts_cluster
    ON gateway_admin_accounts (cluster_id);

CREATE TABLE gateway_admin_project_members (
    cluster_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    proj_id BIGINT NOT NULL,
    role TEXT NOT NULL DEFAULT 'space_admin',
    created_at_ms BIGINT NOT NULL,
    PRIMARY KEY (cluster_id, account_id, proj_id),
    CONSTRAINT chk_gateway_admin_project_members_role
        CHECK (role IN ('space_admin')),
    CONSTRAINT fk_gateway_admin_project_members_account
        FOREIGN KEY (account_id) REFERENCES gateway_admin_accounts (account_id)
        ON DELETE CASCADE
);

CREATE INDEX ix_gateway_admin_project_members_account
    ON gateway_admin_project_members (cluster_id, account_id);
CREATE INDEX ix_gateway_admin_project_members_proj
    ON gateway_admin_project_members (cluster_id, proj_id);

CREATE TABLE gateway_admin_sessions (
    session_id TEXT PRIMARY KEY,
    cluster_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    expires_at_ms BIGINT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    CONSTRAINT fk_gateway_admin_sessions_account
        FOREIGN KEY (account_id) REFERENCES gateway_admin_accounts (account_id)
        ON DELETE CASCADE
);

CREATE UNIQUE INDEX ux_gateway_admin_sessions_token_hash
    ON gateway_admin_sessions (token_hash);
CREATE INDEX ix_gateway_admin_sessions_account
    ON gateway_admin_sessions (cluster_id, account_id);

CREATE TABLE session_inbox_messages (
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
    from_address TEXT,
    in_reply_to TEXT,
    references_json JSONB,
    PRIMARY KEY (cluster_id, message_id)
);

CREATE INDEX idx_session_inbox_queued
    ON session_inbox_messages (cluster_id, session_id, status, created_at_ms, message_id);
CREATE UNIQUE INDEX idx_session_inbox_idempotency
    ON session_inbox_messages (cluster_id, session_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;
