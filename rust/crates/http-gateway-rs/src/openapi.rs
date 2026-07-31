//! Gateway OpenAPI document — derived only from Rust handler/DTO types. Author: kejiqing

use serde_json::{json, Value};
use utoipa::OpenApi;

#[cfg(test)]
const ROUTE_CONTRACT: &str = include_str!("../tests/route_contract.baseline.txt");

/// Sole OpenAPI source: `#[utoipa::path]` on handlers + `ToSchema` on DTOs.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "claw gateway rs",
        version = "0.1.0",
        description = "HTTP gateway API. Request/response schemas are derived from Rust types (utoipa ToSchema / path macros); there is no hand-authored schema document."
    ),
    paths(
        crate::routes::app::admin_mcp_http_handler,
        crate::routes::app::dev_seed_biz_report_task,
        crate::routes::app::get_biz_advice_report,
        crate::routes::app::get_biz_advice_report_bak,
        crate::routes::app::post_agent_feedback,
        crate::routes::app::get_agent_feedback,
        crate::routes::app::get_gateway_global_settings_handler,
        crate::routes::app::reset_gateway_observe_tap_handler,
        crate::routes::app::get_gateway_e2b_singletons_handler,
        crate::routes::app::get_gateway_e2b_templates_handler,
        crate::routes::app::put_gateway_e2b_singleton_templates_handler,
        crate::routes::app::ensure_gateway_e2b_singleton_handler,
        crate::routes::app::reset_gateway_e2b_singleton_handler,
        crate::routes::app::put_gateway_e2b_worker_settings_handler,
        crate::routes::app::put_gateway_claw_tap_handler,
        crate::routes::app::probe_gateway_claw_tap_handler,
        crate::routes::app::put_gateway_strict_landlock_default_handler,
        crate::routes::app::upsert_gateway_git_pat_handler,
        crate::routes::app::delete_gateway_git_pat_handler,
        crate::routes::app::issue_gateway_admin_mcp_token_handler,
        crate::routes::app::revoke_gateway_admin_mcp_token_handler,
        crate::routes::app::put_gateway_active_llm_config_handler,
        crate::routes::app::test_gateway_llm_model_handler,
        crate::routes::app::upsert_gateway_llm_model_handler,
        crate::routes::app::delete_gateway_llm_model_handler,
        crate::routes::app::list_gateway_llm_model_versions_handler,
        crate::routes::app::apply_gateway_llm_model_head_handler,
        crate::routes::app::apply_gateway_llm_model_revision_handler,
        crate::routes::app::healthz,
        crate::routes::app::readyz,
        crate::routes::app::test_mcp,
        crate::routes::app::inject_mcp,
        crate::routes::app::get_injected_mcp,
        crate::routes::app::delete_injected_mcp,
        crate::routes::app::root,
        crate::routes::app::docs,
        crate::routes::app::openapi,
        crate::routes::app::get_preflight_plugins_handler,
        crate::routes::app::put_preflight_plugin_handler,
        crate::routes::app::list_gateway_endpoints_handler,
        crate::routes::app::delete_gateway_endpoint_handler,
        crate::routes::app::delete_claw_pool_handler,
        crate::routes::app::list_claw_pools_handler,
        crate::routes::app::get_project_tools_catalog,
        crate::routes::app::get_project_claude_md,
        crate::routes::app::update_project_claude_md,
        crate::routes::app::upsert_project_skill,
        crate::routes::app::get_effective_prompt,
        crate::routes::app::post_effective_prompt,
        crate::routes::app::list_proj_skills,
        crate::routes::app::get_proj_skill,
        crate::routes::app::list_project_entity_versions,
        crate::routes::app::compare_project_entity_versions,
        crate::routes::app::restore_project_entity_revision,
        crate::routes::app::get_project_config,
        crate::routes::app::list_project_config_versions,
        crate::routes::app::compare_project_config_versions,
        crate::routes::app::activate_project_config_version,
        crate::routes::app::put_project_config,
        crate::routes::app::commit_project_config_draft,
        crate::routes::app::patch_project_config_version_note,
        crate::routes::app::delete_project_config_version,
        crate::routes::app::get_project_e2b_worker_handler,
        crate::routes::app::reset_project_e2b_worker_handler,
        crate::routes::app::get_project_inference_handler,
        crate::routes::app::upsert_project_llm_model_handler,
        crate::routes::app::test_project_llm_model_handler,
        crate::routes::app::delete_project_llm_model_handler,
        crate::routes::app::list_project_llm_model_versions_handler,
        crate::routes::app::apply_project_llm_model_head_handler,
        crate::routes::app::apply_project_llm_model_revision_handler,
        crate::routes::app::get_project_observe_handler,
        crate::routes::app::reset_project_observe_handler,
        crate::routes::app::list_projects,
        crate::routes::app::pull_project_git,
        crate::routes::app::create_project,
        crate::routes::app::patch_project,
        crate::routes::app::delete_project,
        crate::routes::app::init_workspace,
        crate::routes::app::agent_ws_handler,
        crate::routes::app::ovs_workspace_handler,
        crate::routes::app::list_project_sessions,
        crate::routes::app::get_session_execution,
        crate::routes::app::post_gateway_translate,
        crate::routes::app::get_conversation_translate,
        crate::routes::app::rebuild_conversation_translate,
        crate::routes::app::solve,
        crate::routes::app::solve_start,
        crate::routes::app::solve_async,
        crate::routes::app::get_task,
        crate::routes::app::cancel_task,
        crate::routes::app::list_session_turns,
        crate::routes::app::get_turn_tools,
        crate::routes::app::get_turn_timeline,
        crate::routes::app::cancel_session_turn,
        crate::session_upload::upload_session_files
    ),
    components(schemas(
        crate::gateway_e2b_singleton_lifecycle::E2bSingletonComponent,
        crate::project_entity_revision::ProjectEntityDomain
    )),
    tags(
        (name = "System", description = "Gateway health and API metadata"),
        (name = "Projects", description = "Project lifecycle and source synchronization"),
        (name = "Project Config", description = "Versioned project configuration and assets"),
        (name = "ProjectInference", description = "Project inference, models, workers, and observability"),
        (name = "Inference", description = "Project inference, models, workers, and observability"),
        (name = "Sessions", description = "Sessions, turns, execution, files, and reports"),
        (name = "Solve", description = "Synchronous and asynchronous solve execution"),
        (name = "Gateway Settings", description = "Cluster-wide gateway administration"),
        (name = "MCP", description = "MCP injection, probing, and admin operations"),
        (name = "Pools", description = "Pools, endpoints, and preflight plugins")
    )
)]
struct DerivedApi;

/// Build the OpenAPI document exclusively from derived handler/DTO types.
pub fn document() -> Value {
    let mut document =
        serde_json::to_value(DerivedApi::openapi()).expect("serialize derived OpenAPI document");

    document.as_object_mut().expect("OpenAPI root").insert(
        "servers".to_string(),
        json!([{"url": "/", "description": "Current gateway"}]),
    );

    let operation_count = count_operations(&document);
    document
        .as_object_mut()
        .expect("OpenAPI root")
        .insert("x-route-count".to_string(), json!(operation_count));
    document
}

/// Back-compat entry used by `/openapi.json` after the hand-authored base was removed.
pub fn complete(_ignored: Value) -> Value {
    document()
}

fn count_operations(document: &Value) -> u64 {
    document
        .get("paths")
        .and_then(Value::as_object)
        .map(|paths| {
            paths
                .values()
                .map(|item| item.as_object().map(|o| o.len()).unwrap_or(0))
                .sum::<usize>() as u64
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract_operations() -> Vec<(String, String)> {
        ROUTE_CONTRACT
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let mut columns = line.splitn(3, '\t');
                let method = columns.next().expect("method").to_ascii_lowercase();
                let path = columns.next().expect("path").to_string();
                (method, path)
            })
            .collect()
    }

    #[test]
    fn document_covers_verified_route_contract() {
        let document = document();
        let contract = contract_operations();
        assert_eq!(document["x-route-count"], contract.len() as u64);
        let paths = document["paths"].as_object().expect("paths");
        for (method, path) in &contract {
            assert!(
                paths.get(path).and_then(|item| item.get(method)).is_some(),
                "missing derived OpenAPI op for {method} {path}"
            );
        }
    }

    #[test]
    fn translate_schemas_are_derived_from_rust_types() {
        let document = document();
        let operation = &document["paths"]["/v1/gateway/translate"]["post"];
        assert_eq!(
            operation["requestBody"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/GatewayTranslateRequest"
        );
        assert_eq!(
            operation["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/GatewayTranslateResponse"
        );
        let request = &document["components"]["schemas"]["GatewayTranslateRequest"];
        assert_eq!(request["required"], json!(["text"]));
        assert!(request["properties"]["targetLanguage"].is_object());
        assert_eq!(
            document["components"]["schemas"]["GatewayTranslateResponse"]["properties"]
                ["translatedText"]["type"],
            "string"
        );
        // No Swagger additionalProp1 trap
        assert!(
            operation["requestBody"]["content"]["application/json"]["schema"]
                .get("additionalProperties")
                .is_none()
        );
    }

    #[test]
    fn create_project_schemas_are_derived_from_rust_types() {
        let document = document();
        let operation = &document["paths"]["/v1/projects"]["post"];
        let schema_ref = &operation["requestBody"]["content"]["application/json"]["schema"]["$ref"];
        assert_eq!(schema_ref, "#/components/schemas/CreateProjectRequest");
        assert_eq!(
            document["components"]["schemas"]["CreateProjectRequest"]["required"],
            json!(["projectCode"])
        );
    }

    #[test]
    fn agent_feedback_is_a_derived_enum() {
        let document = document();
        let feedback = &document["components"]["schemas"]["AgentFeedbackValue"];
        assert_eq!(feedback["type"], "string");
        assert_eq!(feedback["enum"], json!(["good", "bad"]));
        assert_eq!(
            document["components"]["schemas"]["AgentFeedbackPostRequest"]["properties"]["feedback"]
                ["$ref"],
            "#/components/schemas/AgentFeedbackValue"
        );
    }

    #[test]
    fn closed_path_parameters_are_derived_enums() {
        let document = document();
        assert_eq!(
            document["components"]["schemas"]["E2bSingletonComponent"]["enum"],
            json!(["nas-api", "observe", "ovs"])
        );
        assert_eq!(
            document["components"]["schemas"]["ProjectEntityDomain"]["enum"],
            json!(["rule", "skill", "mcp", "claude", "tools"])
        );
        assert_eq!(
            document["paths"]["/v1/gateway/global-settings/e2b-singletons/{component}/ensure"]
                ["post"]["parameters"][0]["schema"]["$ref"],
            "#/components/schemas/E2bSingletonComponent"
        );
        assert_eq!(
            document["paths"]
                ["/v1/project/config/{proj_id}/entities/{domain}/{entity_key}/versions"]["get"]
                ["parameters"][1]["schema"]["$ref"],
            "#/components/schemas/ProjectEntityDomain"
        );
    }

    #[test]
    fn solve_attachments_and_progress_types_are_derived() {
        let document = document();
        let operation = &document["paths"]["/v1/solve_async"]["post"];
        assert_eq!(
            operation["requestBody"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/SolveRequest"
        );
        assert!(
            operation["requestBody"]["content"]["application/json"]["schema"]
                .get("additionalProperties")
                .is_none()
        );

        let attachments =
            &document["components"]["schemas"]["SolveRequest"]["properties"]["attachments"];
        let items_ref = attachments
            .get("items")
            .and_then(|i| i.get("$ref"))
            .or_else(|| {
                attachments
                    .pointer("/allOf/0/items/$ref")
                    .or_else(|| attachments.pointer("/oneOf/0/items/$ref"))
            });
        assert_eq!(
            items_ref,
            Some(&json!("#/components/schemas/SolveAttachment")),
            "attachments items must ref SolveAttachment, got {attachments}"
        );
        assert_eq!(
            document["components"]["schemas"]["SolveAttachmentKind"]["enum"],
            json!(["image", "document"])
        );
        assert_eq!(
            document["components"]["schemas"]["SolveAttachment"]["properties"]["kind"]["$ref"],
            "#/components/schemas/SolveAttachmentKind"
        );

        let task = &document["components"]["schemas"]["TaskRecord"];
        assert_eq!(
            task["properties"]["progressHistory"]["items"]["$ref"],
            "#/components/schemas/ProgressEvent"
        );
        assert_eq!(
            task["properties"]["todos"]["items"]["$ref"],
            "#/components/schemas/TaskProgressTodo"
        );

        let progress = &document["components"]["schemas"]["SessionExecutionResponse"]["properties"]
            ["progress"];
        let progress_s = progress.to_string();
        assert!(
            progress_s.contains("#/components/schemas/TaskProgressFile"),
            "SessionExecutionResponse.progress must ref TaskProgressFile, got {progress}"
        );
        assert!(document["components"]["schemas"]
            .get("TaskProgressFile")
            .is_some());
        assert_eq!(
            document["components"]["schemas"]["TurnToolsResponse"]["properties"]["tools"]["items"]
                ["$ref"],
            "#/components/schemas/TurnToolRecord"
        );
        assert_eq!(
            document["components"]["schemas"]["SessionFilesUploadResponse"]["properties"]
                ["attachments"]["items"]["$ref"],
            "#/components/schemas/SessionUploadedAttachment"
        );
    }
}
