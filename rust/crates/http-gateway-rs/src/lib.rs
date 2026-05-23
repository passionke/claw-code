//! Library surface (pool + daemon). The `http-gateway-rs` binary links this crate. Author: kejiqing

pub mod biz_advice_report;
pub mod biz_report_pool_proxy;
pub mod biz_report_sse_log;
pub mod claude_tap_health;
pub mod deploy_image;
pub mod gateway_global_settings;
pub mod live_report_audit;
pub mod pool;
pub mod pool_registry;
pub mod project_config_apply;
pub mod project_config_draft;
pub mod project_config_version;
pub mod project_entity_revision;
pub mod project_git_sync;
pub mod project_tools;
pub mod session_db;
pub mod session_execution;
pub mod session_merge;
pub mod task_status;
pub mod turn_id;
pub mod turn_tools_api;
