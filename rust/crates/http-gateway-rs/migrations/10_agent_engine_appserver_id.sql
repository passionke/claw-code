-- Rename leftover Agent engine id codex-acp to harness id appserver. Author: kejiqing
UPDATE gateway_global_settings
SET settings_json = jsonb_set(
      settings_json,
      '{agentEngines,engines}',
      ((settings_json #> '{agentEngines,engines}') - 'codex-acp')
        || jsonb_build_object(
             'appserver',
             COALESCE(
               settings_json #> '{agentEngines,engines,appserver}',
               settings_json #> '{agentEngines,engines,codex-acp}'
             )
           )
    ),
    updated_at_ms = (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint
WHERE settings_json #> '{agentEngines,engines}' ? 'codex-acp';
