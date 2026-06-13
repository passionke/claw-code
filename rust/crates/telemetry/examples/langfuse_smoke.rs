//! `cargo run -p telemetry --example langfuse_smoke` (source repo `.env` first). Author: kejiqing

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

#[tokio::main]
async fn main() {
    load_repo_dotenv();
    if !otel_enabled() {
        eprintln!("CLAW_OTEL_ENABLED or LANGFUSE_* not configured");
        std::process::exit(1);
    }

    eprintln!("init otel…");
    std::env::set_var("OTEL_SERVICE_NAME", "claw-gateway-rs");
    assert!(init_otel_from_env());

    let gateway = OtelSpanGuard::start("http-gateway-rs", "gateway.solve", None).expect("gateway");
    let traceparent = inject_traceparent(gateway.context()).expect("traceparent");
    let _gw = gateway.enter();

    let pool = OtelSpanGuard::start(
        "claw-pool-daemon",
        "pool.exec_solve",
        Some(&context_from_traceparent(&traceparent)),
    )
    .expect("pool");
    let pool_tp = inject_traceparent(pool.context()).expect("pool tp");
    let _pool = pool.enter();

    let worker = OtelSpanGuard::start(
        "gateway-solve-turn",
        "gateway_solve_turn",
        Some(&context_from_traceparent(&pool_tp)),
    )
    .expect("worker");
    worker.set_langfuse_trace_attrs("smoke-session", "smoke-turn", "smoke-request");
    let _worker = worker.enter();

    let llm = OtelSpanGuard::start("api", "llm.chat", Some(worker.context())).expect("llm");
    llm.set_ok();
    gateway.set_ok();
    pool.set_ok();
    worker.set_ok();

    drop(llm);
    drop(worker);
    drop(pool);
    drop(gateway);

    eprintln!("sleep before shutdown…");
    tokio::time::sleep(Duration::from_secs(2)).await;
    eprintln!("shutdown…");
    shutdown_otel();
    eprintln!("done");
}
