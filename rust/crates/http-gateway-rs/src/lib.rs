//! Library surface (pool + daemon). The `http-gateway-rs` binary links this crate. Author: kejiqing
#![allow(
    clippy::assigning_clones,
    clippy::doc_markdown,
    clippy::if_not_else,
    clippy::items_after_statements,
    clippy::map_unwrap_or,
    clippy::option_map_unit_fn,
    clippy::redundant_pattern_matching,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::match_result_ok,
    clippy::redundant_closure_for_method_calls,
    clippy::cast_possible_truncation,
    clippy::must_use_candidate
)]

pub mod admin_mcp_http;
pub mod biz_advice_report;
pub mod biz_report_sse_log;
pub mod claw_tap_cluster_state;
pub mod client_origin;
pub mod cluster_identity;
pub mod deploy_image;
pub mod gateway_admin_mcp_token;
pub mod gateway_claw_tap_lifecycle;
pub mod gateway_claw_tap_settings;
pub mod gateway_e2b_nas_api_settings;
pub mod gateway_e2b_nas_settings;
pub mod gateway_e2b_observe_proxy;
pub mod gateway_e2b_observe_reset;
pub mod gateway_e2b_ovs_settings;
pub mod gateway_e2b_worker_settings;
pub mod gateway_global_settings;
pub mod gateway_llm_cluster_store;
pub mod gateway_llm_config_sync;
pub mod gateway_llm_model_apply;
pub mod gateway_llm_model_revision;
pub mod gateway_strict_landlock_settings;
pub mod gateway_translate;
pub mod live_report_audit;
pub mod llm_probe;
pub mod mcp_probe;
pub mod persistence;
pub mod pool;
pub mod pool_consumer_resolve;
pub mod pool_registry;
pub mod pool_worker_runtime_sync;
pub mod project_config_apply;
pub mod project_config_draft;
pub mod project_config_version;
pub mod project_entity_revision;
pub mod project_extra_session;
pub mod project_git_sync;
pub mod project_id;
pub mod project_tools;
pub mod session_agent_api;
pub mod session_db;
pub mod session_execution;
pub mod session_merge;
pub mod session_ovs_api;
pub mod session_terminal_api;
pub mod solve_llm_route;
pub mod task_status;
pub mod turn_id;
pub mod turn_timeline_api;
pub mod turn_tools_api;
pub mod workspace_perm;
