-- LLM model context window (max input tokens) on Admin cards. Author: kejiqing
ALTER TABLE gateway_llm_cluster_revision
    ADD COLUMN IF NOT EXISTS context_window_tokens INTEGER;

ALTER TABLE gateway_llm_project_revision
    ADD COLUMN IF NOT EXISTS context_window_tokens INTEGER;
