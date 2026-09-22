-- Compact trigger ratio (percent of context_window_tokens) on Admin model cards. Author: kejiqing
ALTER TABLE gateway_llm_cluster_revision
    ADD COLUMN IF NOT EXISTS compact_ratio_percent INTEGER;

ALTER TABLE gateway_llm_project_revision
    ADD COLUMN IF NOT EXISTS compact_ratio_percent INTEGER;
