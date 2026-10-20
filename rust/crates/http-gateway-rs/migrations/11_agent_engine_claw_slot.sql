-- Seed claw Agent engine slot (neuro-harness-acp raw). Replace ref/digest via Admin
-- or migrate-existing-config.sh after publishing neuro-harness-acp. Author: kejiqing
UPDATE gateway_global_settings
SET settings_json = jsonb_set(
      settings_json,
      '{agentEngines,engines,claw}',
      COALESCE(
        settings_json #> '{agentEngines,engines,claw}',
        jsonb_build_object(
          'ref',
          'https://nora.home.passionke.top/raw/claw-agent-engines/neuro-harness-acp-PLACEHOLDER-amd64.tar.gz',
          'digest',
          'sha256:0000000000000000000000000000000000000000000000000000000000000000'
        )
      )
    ),
    updated_at_ms = (EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint
WHERE settings_json ? 'agentEngines'
  AND NOT (settings_json #> '{agentEngines,engines}' ? 'claw');
