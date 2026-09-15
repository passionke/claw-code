//! HTTP drain of gateway session inbox for mid-turn steer. Author: kejiqing

use std::time::Duration;

use reqwest::blocking::Client;
use runtime::{InboxSteerSource, SteerInboxMessage};
use serde_json::Value;

/// Env set by solve_pool when project_role=steerable. Author: kejiqing
pub const ENV_INBOX_ENABLED: &str = "CLAW_INBOX_ENABLED";

/// Build HttpInboxSteer when worker env enables inbox. Author: kejiqing
#[must_use]
pub fn maybe_http_inbox_steer() -> Option<Box<dyn InboxSteerSource>> {
    let enabled = std::env::var(ENV_INBOX_ENABLED)
        .ok()
        .map(|v| {
            let t = v.trim();
            t == "1" || t.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false);
    if !enabled {
        return None;
    }
    let gateway_base = std::env::var("CLAW_GATEWAY_BASE")
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())?;
    let session_id = std::env::var("CLAW_SESSION_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let turn_id = std::env::var("CLAW_TURN_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    let proj_id = std::env::var("CLAW_PROJ_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&id| id >= 1)?;
    Some(Box::new(HttpInboxSteer {
        gateway_base,
        session_id,
        proj_id,
        turn_id,
        client: Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(0)
            .build()
            .ok()?,
    }))
}

struct HttpInboxSteer {
    gateway_base: String,
    session_id: String,
    proj_id: i64,
    turn_id: String,
    client: Client,
}

impl InboxSteerSource for HttpInboxSteer {
    fn drain_before_llm(&mut self, iteration: usize) -> Result<Vec<SteerInboxMessage>, String> {
        let url = format!(
            "{}/v1/sessions/{}/inbox/drain",
            self.gateway_base, self.session_id
        );
        let body = serde_json::json!({
            "projId": self.proj_id,
            "turnId": self.turn_id,
            "iteration": i32::try_from(iteration).unwrap_or(i32::MAX),
        });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .map_err(|e| format!("POST {url}: {e}"))?;
        let status = resp.status();
        let text = resp.text().map_err(|e| format!("read drain body: {e}"))?;
        if !status.is_success() {
            return Err(format!("POST {url} {status}: {text}"));
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("drain json: {e}: {text}"))?;
        let arr = v
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(arr.len());
        for item in arr {
            let message_id = item
                .get("messageId")
                .or_else(|| item.get("message_id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let source = item
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_string();
            let body = item
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if message_id.is_empty() {
                continue;
            }
            let from_address = item
                .get("fromAddress")
                .or_else(|| item.get("from_address"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let in_reply_to = item
                .get("inReplyTo")
                .or_else(|| item.get("in_reply_to"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let references = item
                .get("references")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            out.push(SteerInboxMessage {
                message_id,
                source,
                body,
                from_address,
                in_reply_to,
                references,
            });
        }
        Ok(out)
    }
}
