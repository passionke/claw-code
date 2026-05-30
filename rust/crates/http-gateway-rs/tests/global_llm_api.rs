//! Integration: per-cluster LLM save → DB → runtime sync (no version history). Author: kejiqing

use std::sync::Arc;

use http_gateway_rs::gateway_global_settings::{self, PutActiveLlmConfigInput, PutLlmModelInput};
use http_gateway_rs::gateway_llm_config_sync;
use http_gateway_rs::session_db::GatewaySessionDb;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration, Instant};

fn ensure_test_env(tmp: &std::path::Path) {
    std::env::set_var(
        "CLAW_GATEWAY_DATABASE_URL",
        std::env::var("CLAW_GATEWAY_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://claw_gateway:clawGw9Dev_Pg@127.0.0.1:5433/claw_gateway".into()
        }),
    );
    let test_cluster = format!("test-llm-{}", std::process::id());
    std::env::set_var("CLAW_CLUSTER_ID", &test_cluster);
    let claw_dir = tmp.join(".claw");
    std::fs::create_dir_all(&claw_dir).expect("mkdir .claw");
    let llm_env = claw_dir.join("claw-llm-runtime.env");
    std::env::set_var("CLAW_REPO_ROOT", tmp.display().to_string());
    std::env::set_var("CLAW_LLM_RUNTIME_ENV_FILE", llm_env.display().to_string());
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
async fn global_llm_put_active_roundtrip_and_file_sync() {
    let tmp = tempfile::tempdir().expect("tempdir");
    ensure_test_env(tmp.path());
    let test_cluster = std::env::var("CLAW_CLUSTER_ID").expect("CLAW_CLUSTER_ID");

    // In CI the postgres service may not be ready yet; retry to avoid flaky `PoolTimedOut`.
    // Remote GH actions can take longer to start Postgres than local runs.
    let db_deadline = Instant::now() + Duration::from_secs(90);
    let db = loop {
        match GatewaySessionDb::open().await {
            Ok(db) => break db,
            Err(e) => {
                if Instant::now() >= db_deadline {
                    eprintln!("[global_llm_api] connect retry exhausted: {e}");
                    eprintln!(
                        "[global_llm_api] skip: no reachable PostgreSQL for CLAW_GATEWAY_DATABASE_URL"
                    );
                    return;
                }
                sleep(Duration::from_millis(1000)).await;
            }
        }
    };

    let _ = db.delete_llm_cluster_all(&test_cluster).await;

    let saved = gateway_global_settings::put_active_llm_config(
        &db,
        PutActiveLlmConfigInput {
            name: Some("mock-集成测试".into()),
            base_model_url: "https://api.example.com/v1".into(),
            model_name: "mock-model-v1".into(),
            api_key: Some("sk-mock-global-llm-test".into()),
            note: None,
        },
    )
    .await
    .expect("put_active_llm_config");

    assert_eq!(saved.base_model_url, "https://api.example.com/v1");
    assert_eq!(saved.model_name, "mock-model-v1");
    assert!(saved.api_key_set);
    assert!(!saved.model_id.is_empty());

    let versions = gateway_global_settings::list_llm_model_versions(&db, &saved.model_id)
        .await
        .expect("list versions");
    assert!(
        versions.versions.is_empty(),
        "LLM must not expose version history"
    );

    let handle: gateway_llm_config_sync::LlmRuntimeHandle = Arc::new(RwLock::new(None));
    let sync = gateway_llm_config_sync::sync_llm_runtime_from_db(&db, &handle)
        .await
        .expect("sync runtime");
    assert!(
        sync.changed,
        "sync should update in-memory runtime: {sync:?}"
    );

    let guard = handle.read().await;
    let runtime = guard.as_ref().expect("runtime in memory");
    assert_eq!(runtime.upstream_base_url, "https://api.example.com/v1");
    assert_eq!(runtime.model_name, "mock-model-v1");
    assert_eq!(runtime.api_key, "sk-mock-global-llm-test");
    drop(guard);

    let loaded = gateway_global_settings::load_active_llm_config_public(&db)
        .await
        .expect("load public")
        .expect("active config");
    assert_eq!(loaded.base_model_url, "https://api.example.com/v1");

    let first_id = saved.model_id.clone();

    gateway_global_settings::upsert_llm_model(
        &db,
        PutLlmModelInput {
            id: Some(first_id.clone()),
            name: "mock-集成测试".into(),
            base_model_url: "https://api.example.com/v1".into(),
            model_name: "mock-model-v2".into(),
            api_key: None,
            note: None,
        },
    )
    .await
    .expect("upsert without new api key");

    let second = gateway_global_settings::upsert_llm_model(
        &db,
        PutLlmModelInput {
            id: None,
            name: "mock-第二模型".into(),
            base_model_url: "https://api.example.com/v1".into(),
            model_name: "mock-model-alt".into(),
            api_key: Some("sk-mock-alt".into()),
            note: None,
        },
    )
    .await
    .expect("create second model");
    assert_ne!(second.id, first_id);

    gateway_global_settings::apply_llm_model_by_id(&db, &second.id, None)
        .await
        .expect("apply second");
    let active = gateway_global_settings::load_active_llm_config_public(&db)
        .await
        .expect("load")
        .expect("active");
    assert_eq!(active.model_id, second.id);
    assert_eq!(active.model_name, "mock-model-alt");

    let _ = db.delete_llm_cluster_all(&test_cluster).await;
}
