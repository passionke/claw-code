-- gateway_model_usage base_url column: observe tap records the upstream LLM base URL per call.
-- Author: kejiqing
ALTER TABLE gateway_model_usage
    ADD COLUMN IF NOT EXISTS base_url TEXT;
