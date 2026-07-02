-- Global preflight plugin registry (Admin). Author: kejiqing
CREATE TABLE IF NOT EXISTS preflight_plugin (
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
