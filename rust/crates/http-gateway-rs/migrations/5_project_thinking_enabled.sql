-- Project-level LLM thinking switch (default off). Author: kejiqing
ALTER TABLE project_config
    ADD COLUMN IF NOT EXISTS thinking_enabled BOOLEAN NOT NULL DEFAULT false;
