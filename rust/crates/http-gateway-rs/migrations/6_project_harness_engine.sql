-- Project harness engine, fixed at creation (feat/neuro-harness; renumber when merging to main).
-- Author: kejiqing
ALTER TABLE project_config
    ADD COLUMN IF NOT EXISTS harness_engine TEXT NOT NULL DEFAULT 'claw'
        CONSTRAINT project_config_harness_engine_check
        CHECK (harness_engine IN ('claw', 'appserver', 'opencode'));
