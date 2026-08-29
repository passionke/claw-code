//! Integration: cluster bootstrap apply LLM from env. Author: kejiqing

use std::sync::Arc;

use http_gateway_rs::gateway_cluster_bootstrap;
use http_gateway_rs::gateway_global_settings;
use http_gateway_rs::session_db::try_open_integration_database;
use tokio::sync::RwLock;

#[tokio::test]
async fn bootstrap_apply_llm_from_env_roundtrip() {
    let test_cluster = format!("test-bootstrap-{}", std::process::id());
    std::env::set_var("CLAW_CLUSTER_ID", &test_cluster);
    std::env::set_var("CLAW_BOOTSTRAP_LLM_API_KEY", "sk-bootstrap-test");
    std::env::set_var("CLAW_BOOTSTRAP_LLM_BASE_URL", "https://api.example.com/v1");
    std::env::set_var("CLAW_BOOTSTRAP_LLM_MODEL_NAME", "bootstrap-mock");

    let Some(db) = try_open_integration_database().await else {
        eprintln!("[cluster_bootstrap_api] skip: PostgreSQL not configured or not reachable");
        std::env::remove_var("CLAW_BOOTSTRAP_LLM_API_KEY");
        std::env::remove_var("CLAW_BOOTSTRAP_LLM_BASE_URL");
        std::env::remove_var("CLAW_BOOTSTRAP_LLM_MODEL_NAME");
        return;
    };

    let _ = db.delete_llm_cluster_all(&test_cluster).await;

    let llm_handle: http_gateway_rs::gateway_llm_config_sync::LlmRuntimeHandle =
        Arc::new(RwLock::new(None));

    let resp = gateway_cluster_bootstrap::apply_llm_from_env(&db, &llm_handle)
        .await
        .expect("apply_llm_from_env");
    assert!(resp.applied, "expected applied: {:?}", resp.message);

    let active = gateway_global_settings::load_active_llm_runtime(&db)
        .await
        .expect("load active")
        .expect("active llm");
    assert_eq!(active.model_name, "bootstrap-mock");

    let snap = gateway_cluster_bootstrap::cluster_bootstrap_status(&db, None, None)
        .await
        .expect("status");
    assert!(snap.phases.iter().any(|p| {
        p.phase == gateway_cluster_bootstrap::BootstrapPhaseId::LlmConfig && p.complete
    }));

    std::env::remove_var("CLAW_BOOTSTRAP_LLM_API_KEY");
    std::env::remove_var("CLAW_BOOTSTRAP_LLM_BASE_URL");
    std::env::remove_var("CLAW_BOOTSTRAP_LLM_MODEL_NAME");
}
