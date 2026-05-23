//! Global gateway LLM config (single row, no version history). Author: kejiqing

use chrono::{Local, TimeZone};

/// Single global LLM slot in PG + runtime files. Author: kejiqing
pub const GLOBAL_LLM_MODEL_ID: &str = "global";
pub const GLOBAL_LLM_REV: &str = "global";

/// Api-key map slot: `{model_id}@{model_rev}`. Author: kejiqing
#[must_use]
pub fn llm_api_key_slot(model_id: &str, model_rev: &str) -> String {
    format!("{model_id}@{model_rev}")
}

/// Formal revision id: local `YYYY-MM-DD_HH-mm-ss`. Author: kejiqing
#[must_use]
pub fn format_model_rev_local_ms(ms: i64) -> String {
    let Some(dt) = Local.timestamp_millis_opt(ms).single() else {
        return ms.to_string();
    };
    dt.format("%Y-%m-%d_%H-%M-%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_slot_format() {
        assert_eq!(
            llm_api_key_slot("llm-a", "2026-05-23_12-00-00"),
            "llm-a@2026-05-23_12-00-00"
        );
    }
}
