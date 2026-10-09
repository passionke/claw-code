-- Router hub config (bodyRelay etc.). Author: kejiqing

ALTER TABLE project_config
    ADD COLUMN IF NOT EXISTS router_json JSONB NOT NULL DEFAULT '{}'::jsonb;
