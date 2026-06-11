//! Manual smoke: distributed trace export to Langfuse. Author: kejiqing
//!
//! Run from repo root:
//!   set -a && source .env && set +a
//!   cargo test -p telemetry --test langfuse_otel_smoke -- --ignored --nocapture

use std::time::Duration;

use telemetry::{
    context_from_traceparent, init_otel_from_env, inject_traceparent, otel_enabled, shutdown_otel,
    OtelSpanGuard,
};

fn load_repo_dotenv() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dotenv = manifest.join("../../.env");
    if dotenv.exists() {
        for line in std::fs::read_to_string(&dotenv).unwrap_or_default().lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if std::env::var(key).is_err() {
                std::env::set_var(key, value);
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires LANGFUSE_* in .env and reachable Langfuse OTLP endpoint"]
async fn export_distributed_solve_trace_chain() {
    load_repo_dotenv();
    if !otel_enabled() {
        eprintln!("skip: CLAW_OTEL_ENABLED or LANGFUSE_* not configured");
        return;
    }

    std::env::set_var("OTEL_SERVICE_NAME", "claw-gateway-rs");
    assert!(init_otel_from_env(), "init_otel_from_env failed");

    let gateway =
        OtelSpanGuard::start("http-gateway-rs", "gateway.solve", None).expect("gateway span");
    gateway.set_attribute("session_id", "smoke-session");
    gateway.set_attribute("turn_id", "smoke-turn");
    gateway.set_attribute("request_id", "smoke-request");
    let traceparent = inject_traceparent(gateway.context()).expect("traceparent");
    let _gw_enter = gateway.enter();

    std::env::set_var("OTEL_SERVICE_NAME", "claw-pool-daemon");
    let parent = context_from_traceparent(&traceparent);
    let pool = OtelSpanGuard::start("claw-pool-daemon", "pool.exec_solve", Some(&parent))
        .expect("pool span");
    pool.set_attribute("session_id", "smoke-session");
    let pool_tp = inject_traceparent(pool.context()).expect("pool traceparent");
    let _pool_enter = pool.enter();

    std::env::set_var("OTEL_SERVICE_NAME", "claw-worker");
    let worker_parent = context_from_traceparent(&pool_tp);
    let worker = OtelSpanGuard::start(
        "gateway-solve-turn",
        "gateway_solve_turn",
        Some(&worker_parent),
    )
    .expect("worker span");
    worker.set_langfuse_trace_attrs("smoke-session", "smoke-turn", "smoke-request");
    let _worker_enter = worker.enter();

    let llm = OtelSpanGuard::start("api", "llm.chat", Some(worker.context())).expect("llm span");
    llm.set_attribute("gen_ai.system", "anthropic");
    llm.set_attribute("gen_ai.request.model", "smoke-model");
    llm.set_ok();

    worker.set_ok();
    pool.set_ok();
    gateway.set_ok();

    drop(llm);
    drop(worker);
    drop(pool);
    drop(gateway);

    tokio::time::sleep(Duration::from_secs(3)).await;
    eprintln!("smoke: flushing OTEL batch export…");
    let shutdown = tokio::task::spawn_blocking(shutdown_otel);
    let _ = tokio::time::timeout(Duration::from_secs(10), shutdown).await;
    eprintln!("smoke: exported gateway.solve → pool.exec_solve → gateway_solve_turn → llm.chat");
}
