//! HTTP client for http-gateway-rs (L2). Author: kejiqing

use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone)]
pub struct GatewayClient {
    base: String,
    http: Client,
}

#[derive(Debug, Deserialize)]
pub struct SolveAsyncResponse {
    #[serde(rename = "taskId")]
    pub task_id: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct TaskRecord {
    pub status: String,
    pub result: Option<Value>,
    pub error: Option<Value>,
}

impl GatewayClient {
    pub fn from_env() -> Self {
        let base = std::env::var("CLAW_GATEWAY_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
        Self {
            base: base.trim_end_matches('/').to_string(),
            http: Client::builder()
                .timeout(Duration::from_secs(600))
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn solve_async(
        &self,
        ds_id: i64,
        user_prompt: &str,
        body_session_id: Option<&str>,
        claw_session_header: &str,
        extra_session: Option<Value>,
        run_id: &str,
    ) -> Result<SolveAsyncResponse, String> {
        let mut body = json!({
            "dsId": ds_id,
            "userPrompt": user_prompt,
        });
        if let Some(sid) = body_session_id {
            body["sessionId"] = json!(sid);
        }
        if let Some(extra) = extra_session {
            body["extraSession"] = extra;
        }
        let resp = self
            .http
            .post(format!("{}/v1/solve_async", self.base))
            .header("x-request-id", run_id)
            .header("claw-session-id", claw_session_header)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("solve_async {status}: {text}"));
        }
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn get_task(&self, task_id: &str) -> Result<TaskRecord, String> {
        let resp = self
            .http
            .get(format!("{}/v1/tasks/{task_id}", self.base))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("get_task {}", resp.status()));
        }
        resp.json().await.map_err(|e| e.to_string())
    }

    pub async fn fetch_event_lines(&self, task_id: &str) -> Result<Vec<Value>, String> {
        let resp = self
            .http
            .get(format!("{}/v1/events/{task_id}", self.base))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("events {}", resp.status()));
        }
        let text = resp.text().await.map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                out.push(v);
            }
        }
        Ok(out)
    }

    pub async fn resolve_interrupt(
        &self,
        interrupt_id: &str,
        decision: &str,
        answer: Option<&str>,
    ) -> Result<(), String> {
        let body = json!({
            "decision": decision,
            "answer": answer,
        });
        let resp = self
            .http
            .post(format!(
                "{}/v1/interrupts/{interrupt_id}/resolve",
                self.base
            ))
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("resolve_interrupt {}", resp.status()));
        }
        Ok(())
    }
}
