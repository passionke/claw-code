-- Project harness engine, fixed at creation (feat/neuro-harness; was 6 on branch, renumbered to 7 for main).
-- Author: kejiqing
ALTER TABLE project_config
    ADD COLUMN IF NOT EXISTS harness_engine TEXT NOT NULL DEFAULT 'claw'
        CONSTRAINT project_config_harness_engine_check
        CHECK (harness_engine IN ('claw', 'appserver', 'opencode'));
