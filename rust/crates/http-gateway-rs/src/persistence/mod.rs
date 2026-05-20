//! Persistence: runtime writes session jsonl; flush to PG on solve end; handoff reads PG. Author: kejiqing

pub mod transcript;

pub use transcript::{
    ensure_jsonl_from_db, import_turn_messages_to_db, now_ms, persist_turn_after_solve,
    report_body_from_turn_messages, JsonlMessage,
};
