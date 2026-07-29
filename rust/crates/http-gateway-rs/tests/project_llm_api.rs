//! Integration: project LLM override (proj1) → observe URLs → fallback to global.
//! Author: kejiqing
//!
//! Optional PG: skipped when `CLAW_GATEWAY_DATABASE_URL` unset or host unreachable.

use http_gateway_rs::gateway_global_settings::{self, PutLlmModelInput};
use http_gateway_rs::gateway_project_llm::{self, ProjectLlmMode};
use http_gateway_rs::gateway_project_observe;
use http_gateway_rs::pool::{prepare_e2b_worker_llm_material, PrepareE2bWorkerLlmOptions};
use http_gateway_rs::session_db::try_open_integration_database;

const PROJ1: i64 = 1;
const PROJ2: i64 = 2;

fn ensure_test_env(tmp: &std::path::Path) {
    let test_cluster = format!("test-proj-llm-{}", std::process::id());
    std::env::set_var("CLAW_CLUSTER_ID", &test_cluster);
    let claw_dir = tmp.join(".claw");
    std::fs::create_dir_all(&claw_dir).expect("mkdir .claw");
    std::env::set_var("CLAW_REPO_ROOT", tmp.display().to_string());
    std::env::set_var(
        "CLAW_LLM_RUNTIME_ENV_FILE",
        claw_dir.join("claw-llm-runtime.env").display().to_string(),
    );
    std::env::set_var(
        "CLAW_TAP_UPSTREAM_CONFIG_FILE",
        claw_dir
            .join("claw-tap-upstream.json")
            .display()
            .to_string(),
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn proj1_qwen_plus_then_max_then_inherit_global() {
    let tmp = tempfile::tempdir().expect("tempdir");
    ensure_test_env(tmp.path());
    let test_cluster = std::env::var("CLAW_CLUSTER_ID").expect("CLAW_CLUSTER_ID");

    let Some(db) = try_open_integration_database().await else {
        eprintln!(
            "[project_llm_api] skip: PostgreSQL not configured or not reachable \
             (set CLAW_GATEWAY_DATABASE_URL to run)"
        );
        return;
    };

    let _ = db.delete_llm_cluster_all(&test_cluster).await;
    let _ = db.delete_llm_project_all(&test_cluster, PROJ1).await;
    let _ = db.delete_llm_project_all(&test_cluster, PROJ2).await;

    // Global default model.
    let global = gateway_global_settings::upsert_llm_model(
        &db,
        PutLlmModelInput {
            id: Some("global-default".into()),
            name: "全局默认".into(),
            base_model_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            model_name: "qwen-global-default".into(),
            supports_vision: false,
            api_key: Some("sk-global-mock".into()),
            note: None,
        },
    )
    .await
    .expect("upsert global");
    gateway_global_settings::apply_llm_model_by_id(&db, &global.id, None)
        .await
        .expect("apply global");

    // Seed global clawTap so inherit path can resolve observe proxy without e2b.
    db.merge_gateway_global_settings_json(
        &["clawTap"],
        &serde_json::json!({
            "mode": "remote",
            "host": "8080-global-observe.test",
            "proxyPort": 8080,
            "proxyBaseUrl": "http://8080-global-observe.test",
            "e2bObserveSandboxId": "sbx_global_observe",
            "updatedAtMs": 1
        }),
    )
    .await
    .expect("seed clawTap");

    // 1) proj1 initial → inherit
    let s0 = gateway_project_llm::load_project_inference_settings(&db, PROJ1)
        .await
        .expect("settings inherit");
    assert_eq!(s0.mode, ProjectLlmMode::Inherit);
    assert!(s0.active_llm_config.is_none());

    let material_inherit =
        prepare_e2b_worker_llm_material(&db, PROJ1, None, PrepareE2bWorkerLlmOptions::default())
            .await
            .expect("material inherit");
    assert_eq!(material_inherit.route.mode, "e2bObserveTap");
    assert_eq!(
        material_inherit
            .env
            .get("OPENAI_BASE_URL")
            .map(String::as_str),
        Some("http://8080-global-observe.test")
    );
    assert!(material_inherit.model.contains("qwen-global-default"));

    // 2) Configure qwen3-plus on proj1
    let plus = gateway_project_llm::upsert_project_llm_model(
        &db,
        PROJ1,
        PutLlmModelInput {
            id: Some("proj1-qwen3-plus".into()),
            name: "proj1 qwen3-plus".into(),
            base_model_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            model_name: "qwen3-plus".into(),
            supports_vision: true,
            api_key: Some("sk-proj1-plus".into()),
            note: None,
        },
    )
    .await
    .expect("upsert qwen3-plus");
    gateway_project_llm::apply_project_llm_model_by_id(&db, PROJ1, &plus.id, None)
        .await
        .expect("apply qwen3-plus");

    gateway_project_observe::persist_project_observe_urls_for_test(
        &db,
        &test_cluster,
        PROJ1,
        "sbx_proj1_observe",
        "http://8080-proj1-observe.test",
    )
    .await
    .expect("persist proj1 observe");

    let s1 = gateway_project_llm::load_project_inference_settings(&db, PROJ1)
        .await
        .expect("settings override plus");
    assert_eq!(s1.mode, ProjectLlmMode::Override);
    assert_eq!(
        s1.active_llm_config.as_ref().map(|c| c.model_name.as_str()),
        Some("qwen3-plus")
    );
    assert_eq!(
        s1.observe.proxy_base_url.as_deref(),
        Some("http://8080-proj1-observe.test")
    );

    let material_plus =
        prepare_e2b_worker_llm_material(&db, PROJ1, None, PrepareE2bWorkerLlmOptions::default())
            .await
            .expect("material plus");
    assert_eq!(material_plus.route.mode, "e2bProjectObserveTap");
    assert_eq!(
        material_plus.env.get("OPENAI_BASE_URL").map(String::as_str),
        Some("http://8080-proj1-observe.test")
    );
    assert!(
        material_plus.model.contains("qwen3-plus"),
        "model={}",
        material_plus.model
    );

    // Active runtime exposes key for observe-side consumption (proxy health path is PG-backed).
    let runtime_plus = gateway_project_llm::load_active_project_llm_runtime(&db, PROJ1)
        .await
        .expect("load runtime")
        .expect("active runtime");
    assert_eq!(runtime_plus.model_name, "qwen3-plus");
    assert_eq!(runtime_plus.api_key, "sk-proj1-plus");
    assert!(runtime_plus.supports_vision);
    let effective_proj1 = gateway_project_llm::load_effective_llm_runtime(&db, PROJ1)
        .await
        .expect("load effective proj1")
        .expect("proj1 effective runtime");
    assert!(
        effective_proj1.supports_vision,
        "project override must determine the effective vision capability"
    );

    // 3) Switch to qwen3.7-max (same observe proxy)
    let max = gateway_project_llm::upsert_project_llm_model(
        &db,
        PROJ1,
        PutLlmModelInput {
            id: Some("proj1-qwen3-7-max".into()),
            name: "proj1 qwen3.7-max".into(),
            base_model_url: "https://dashscope.aliyuncs.com/compatible-mode/v1".into(),
            model_name: "qwen3.7-max".into(),
            supports_vision: false,
            api_key: Some("sk-proj1-max".into()),
            note: None,
        },
    )
    .await
    .expect("upsert qwen3.7-max");
    gateway_project_llm::apply_project_llm_model_by_id(&db, PROJ1, &max.id, None)
        .await
        .expect("apply qwen3.7-max");

    let s2 = gateway_project_llm::load_project_inference_settings(&db, PROJ1)
        .await
        .expect("settings max");
    assert_eq!(s2.mode, ProjectLlmMode::Override);
    assert_eq!(
        s2.active_llm_config.as_ref().map(|c| c.model_name.as_str()),
        Some("qwen3.7-max")
    );
    assert_eq!(
        s2.observe.proxy_base_url.as_deref(),
        Some("http://8080-proj1-observe.test"),
        "switching model must keep same project observe"
    );

    let material_max =
        prepare_e2b_worker_llm_material(&db, PROJ1, None, PrepareE2bWorkerLlmOptions::default())
            .await
            .expect("material max");
    assert_eq!(material_max.route.mode, "e2bProjectObserveTap");
    assert!(
        material_max.model.contains("qwen3.7-max"),
        "model={}",
        material_max.model
    );

    // 4) Isolation: proj2 still inherit; global unchanged
    let s_proj2 = gateway_project_llm::load_project_inference_settings(&db, PROJ2)
        .await
        .expect("proj2 inherit");
    assert_eq!(s_proj2.mode, ProjectLlmMode::Inherit);
    let global_active = gateway_global_settings::load_active_llm_config_public(&db)
        .await
        .expect("global active")
        .expect("global has active");
    assert_eq!(global_active.model_name, "qwen-global-default");
    let effective_proj2 = gateway_project_llm::load_effective_llm_runtime(&db, PROJ2)
        .await
        .expect("load effective proj2")
        .expect("proj2 effective runtime");
    assert!(
        !effective_proj2.supports_vision,
        "project without override must inherit global vision capability"
    );

    // 5) Delete all project models → inherit + observe row cleared
    let models: Vec<_> = s2.llm_models.iter().map(|m| m.id.clone()).collect();
    for id in models {
        let (deleted, inherit_now) = gateway_project_llm::delete_project_llm_model(&db, PROJ1, &id)
            .await
            .expect("delete");
        assert!(deleted);
        if inherit_now {
            db.delete_llm_project_observe(&test_cluster, PROJ1)
                .await
                .expect("teardown observe row");
        }
    }

    let s3 = gateway_project_llm::load_project_inference_settings(&db, PROJ1)
        .await
        .expect("settings after delete");
    assert_eq!(s3.mode, ProjectLlmMode::Inherit);
    assert!(!s3.observe.configured);
    assert!(s3.active_llm_config.is_none());

    let material_back =
        prepare_e2b_worker_llm_material(&db, PROJ1, None, PrepareE2bWorkerLlmOptions::default())
            .await
            .expect("material back to global");
    assert_eq!(material_back.route.mode, "e2bObserveTap");
    assert_eq!(
        material_back.env.get("OPENAI_BASE_URL").map(String::as_str),
        Some("http://8080-global-observe.test")
    );
    assert!(material_back.model.contains("qwen-global-default"));

    let _ = db.delete_llm_project_all(&test_cluster, PROJ1).await;
    let _ = db.delete_llm_cluster_all(&test_cluster).await;
}
