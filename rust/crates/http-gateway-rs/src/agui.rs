//! AG-UI event tap and interrupt resolve (L2/L4). Author: kejiqing
#![allow(clippy::must_use_candidate, clippy::unused_async)]

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct EventTapHub {
    lines: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl EventTapHub {
    pub fn push(&self, task_id: &str, event: &Value) {
        let line = event.to_string();
        let mut map = self.lines.lock().expect("event tap lock");
        map.entry(task_id.to_string()).or_default().push(line);
    }

    pub fn snapshot(&self, task_id: &str) -> Vec<String> {
        self.lines
            .lock()
            .expect("event tap lock")
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Clone, Default)]
pub struct InterruptHub {
    pending: Arc<RwLock<HashMap<String, tokio::sync::oneshot::Sender<InterruptDecision>>>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InterruptDecision {
    pub decision: String,
    #[serde(default)]
    pub answer: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InterruptResolveBody {
    pub decision: String,
    #[serde(default)]
    pub answer: Option<String>,
}

impl InterruptHub {
    pub async fn register(&self, interrupt_id: String) -> tokio::sync::oneshot::Receiver<InterruptDecision> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.write().await.insert(interrupt_id, tx);
        rx
    }

    pub async fn resolve(&self, interrupt_id: &str, body: InterruptResolveBody) -> bool {
        let tx = self.pending.write().await.remove(interrupt_id);
        if let Some(tx) = tx {
            let _ = tx.send(InterruptDecision {
                decision: body.decision,
                answer: body.answer,
            });
            true
        } else {
            false
        }
    }
}

pub fn tap_line(task_id: &str, event_type: &str, extra: &Value) -> Value {
    let mut v = json!({ "type": event_type, "taskId": task_id });
    if let Some(obj) = v.as_object_mut() {
        if let Some(ext) = extra.as_object() {
            for (k, val) in ext {
                obj.insert(k.clone(), val.clone());
            }
        }
    }
    v
}

/// `GET /v1/events/{task_id}` — NDJSON snapshot for bridge (L2).
pub async fn get_events(
    Path(task_id): Path<String>,
    State(hub): State<EventTapHub>,
) -> impl IntoResponse {
    let lines = hub.snapshot(&task_id);
    let body = lines.join("\n");
    let body = if body.is_empty() {
        String::new()
    } else {
        format!("{body}\n")
    };
    (
        [(axum::http::header::CONTENT_TYPE, "application/x-ndjson")],
        body,
    )
}

/// `POST /v1/interrupts/{interrupt_id}/resolve` (L4).
pub async fn resolve_interrupt(
    Path(interrupt_id): Path<String>,
    State(hub): State<InterruptHub>,
    Json(body): Json<InterruptResolveBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if hub.resolve(&interrupt_id, body).await {
        Ok(Json(json!({"ok": true, "interruptId": interrupt_id})))
    } else {
        Err((StatusCode::NOT_FOUND, "unknown interrupt".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn interrupt_resolve_roundtrip() {
        let hub = InterruptHub::default();
        let rx = hub.register("i1".to_string()).await;
        assert!(
            hub.resolve(
                "i1",
                InterruptResolveBody {
                    decision: "allow_once".to_string(),
                    answer: None,
                }
            )
            .await
        );
        let dec = rx.await.expect("decision");
        assert_eq!(dec.decision, "allow_once");
    }

    #[test]
    fn event_tap_push_and_snapshot() {
        let hub = EventTapHub::default();
        hub.push("t1", &json!({"type":"solve.queued"}));
        let snap = hub.snapshot("t1");
        assert_eq!(snap.len(), 1);
        assert!(snap[0].contains("solve.queued"));
    }
}
