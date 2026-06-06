//! Bridge ↔ mock gateway HTTP (L2 integration, no Podman). Author: kejiqing

use axum::extract::Path;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;

struct MockGateway {
    events: Mutex<HashMap<String, Vec<String>>>,
    tasks: Mutex<HashMap<String, Value>>,
}

impl MockGateway {
    fn push_event(&self, task_id: &str, line: Value) {
        self.events
            .lock()
            .expect("lock")
            .entry(task_id.to_string())
            .or_default()
            .push(line.to_string());
    }
}

fn mock_router(state: Arc<MockGateway>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { Json(json!({"ok": true})) }))
        .route(
            "/v1/solve_async",
            post({
                let state = Arc::clone(&state);
                move |Json(body): Json<Value>| {
                    let state = Arc::clone(&state);
                    async move {
                        let tid = body
                            .get("sessionId")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .unwrap_or_else(|| "mock-task-1".to_string());
                        let prompt = body
                            .get("userPrompt")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        state.events.lock().expect("lock").remove(&tid);
                        state.push_event(
                            &tid,
                            json!({"type":"solve.queued","taskId": tid, "dsId": 1}),
                        );
                        let reply = if prompt.contains("笑话") || prompt.contains("joke") {
                            "mock joke"
                        } else {
                            "hello from mock gateway"
                        };
                        state.push_event(&tid, json!({"type":"text.delta","text": reply}));
                        state.tasks.lock().expect("lock").insert(
                            tid.clone(),
                            json!({
                                "status": "succeeded",
                                "result": {
                                    "outputText": reply,
                                    "sessionId": tid,
                                }
                            }),
                        );
                        state.push_event(
                            &tid,
                            json!({"type":"solve.finished","status":"succeeded"}),
                        );
                        Json(json!({
                            "taskId": tid,
                            "sessionId": tid,
                            "requestId": tid,
                            "status": "queued",
                            "pollUrl": format!("/v1/tasks/{tid}"),
                        }))
                    }
                }
            }),
        )
        .route(
            "/v1/tasks/{task_id}",
            get({
                let state = Arc::clone(&state);
                move |Path(task_id): Path<String>| {
                    let state = Arc::clone(&state);
                    async move {
                        let tasks = state.tasks.lock().expect("lock");
                        let rec = tasks
                            .get(&task_id)
                            .cloned()
                            .unwrap_or_else(|| json!({"status": "failed"}));
                        Json(rec)
                    }
                }
            }),
        )
        .route(
            "/v1/events/{task_id}",
            get({
                let state = Arc::clone(&state);
                move |Path(task_id): Path<String>| {
                    let state = Arc::clone(&state);
                    async move {
                        let lines = state
                            .events
                            .lock()
                            .expect("lock")
                            .get(&task_id)
                            .cloned()
                            .unwrap_or_default();
                        let body = if lines.is_empty() {
                            String::new()
                        } else {
                            format!("{}\n", lines.join("\n"))
                        };
                        body
                    }
                }
            }),
        )
        .with_state(state)
}

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    listener.local_addr().expect("addr").port()
}

#[tokio::test]
async fn bridge_agent_run_against_mock_gateway() {
    let gw_state = Arc::new(MockGateway {
        events: Mutex::new(HashMap::new()),
        tasks: Mutex::new(HashMap::new()),
    });
    let gw_port = free_port().await;
    let gw_app = mock_router(Arc::clone(&gw_state));
    let gw_listener = TcpListener::bind(format!("127.0.0.1:{gw_port}"))
        .await
        .expect("gw bind");
    tokio::spawn(async move {
        axum::serve(gw_listener, gw_app).await.ok();
    });

    let bridge_port = free_port().await;
    std::env::set_var("CLAW_AGUI_MOCK", "0");
    std::env::set_var(
        "CLAW_GATEWAY_BASE_URL",
        format!("http://127.0.0.1:{gw_port}"),
    );
    std::env::set_var("CLAW_AGUI_BRIDGE_ADDR", format!("127.0.0.1:{bridge_port}"));
    let bridge_handle = tokio::spawn(async move {
        ag_ui_claw_bridge::serve(&format!("127.0.0.1:{bridge_port}"))
            .await
            .ok();
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://127.0.0.1:{bridge_port}/v1/agent/run"))
        .json(&json!({
        "threadId": "mock-thread",
        "runId": "mock-run",
        "messages": [{"role": "user", "content": "ping"}],
        "tools": [],
            "forwardedProps": {"dsId": 1, "sessionId": "mock-thread"}
        }))
        .send()
        .await
        .expect("post");
    assert!(resp.status().is_success());
    let body = resp.text().await.expect("body");
    assert!(body.contains("RUN_STARTED"), "body: {body}");
    assert!(
        body.contains("RUN_FINISHED") || body.contains("hello from mock gateway"),
        "body: {body}"
    );

    bridge_handle.abort();
}

#[tokio::test]
async fn bridge_second_turn_does_not_replay_first_reply() {
    let gw_state = Arc::new(MockGateway {
        events: Mutex::new(HashMap::new()),
        tasks: Mutex::new(HashMap::new()),
    });
    let gw_port = free_port().await;
    let gw_app = mock_router(Arc::clone(&gw_state));
    let gw_listener = TcpListener::bind(format!("127.0.0.1:{gw_port}"))
        .await
        .expect("gw bind");
    tokio::spawn(async move {
        axum::serve(gw_listener, gw_app).await.ok();
    });

    let bridge_port = free_port().await;
    std::env::set_var("CLAW_AGUI_MOCK", "0");
    std::env::set_var(
        "CLAW_GATEWAY_BASE_URL",
        format!("http://127.0.0.1:{gw_port}"),
    );
    std::env::set_var("CLAW_AGUI_BRIDGE_ADDR", format!("127.0.0.1:{bridge_port}"));
    let bridge_handle = tokio::spawn(async move {
        ag_ui_claw_bridge::serve(&format!("127.0.0.1:{bridge_port}"))
            .await
            .ok();
    });
    tokio::time::sleep(Duration::from_millis(200)).await;

    let client = reqwest::Client::new();
    let thread_id = "session-two-turn";
    for (run_id, prompt, must_contain, must_not) in [
        ("run-1", "你好", "hello from mock gateway", "mock joke"),
        ("run-2", "说个笑话", "mock joke", "hello from mock gateway"),
    ] {
        let resp = client
            .post(format!("http://127.0.0.1:{bridge_port}/v1/agent/run"))
            .json(&json!({
                "threadId": thread_id,
                "runId": run_id,
                "messages": [{"role": "user", "content": prompt}],
                "tools": [],
                "forwardedProps": {"dsId": 1, "sessionId": thread_id}
            }))
            .send()
            .await
            .expect("post");
        assert!(resp.status().is_success());
        let body = resp.text().await.expect("body");
        assert!(
            body.contains(must_contain),
            "run {run_id} expected {must_contain}, body: {body}"
        );
        assert!(
            !body.contains(must_not),
            "run {run_id} must not replay {must_not}, body: {body}"
        );
    }

    bridge_handle.abort();
}
