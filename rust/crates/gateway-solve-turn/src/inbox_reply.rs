//! `inbox_reply` tool: send to peer mailbox address. Author: kejiqing

use std::time::Duration;

use api::ToolDefinition;
use reqwest::blocking::Client;
use runtime::ToolError;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::inbox_address::{parse_mailbox_address, MailboxAddress};

pub const INBOX_REPLY_TOOL_NAME: &str = "inbox_reply";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboxReplyInput {
    /// Peer address `sessionId@projId.clusterId`.
    pub to: String,
    pub body: String,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Option<Vec<String>>,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[must_use]
pub fn inbox_reply_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: INBOX_REPLY_TOOL_NAME.to_string(),
        description: Some(
            "Reply to another agent's gateway inbox. \
             `to` must be sessionId@projId.clusterId (never omit sessionId). \
             Always set `inReplyTo` to the message id you are answering; \
             when you only used a subset of drained steers, list their ids in `references`."
                .to_string(),
        ),
        input_schema: json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient mailbox address: sessionId@projId.clusterId"
                },
                "body": {
                    "type": "string",
                    "description": "Reply body (utf-8)"
                },
                "inReplyTo": {
                    "type": "string",
                    "description": "messageId of the primary steer/message being answered"
                },
                "references": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "messageIds this reply is based on (e.g. first N of a larger drain)"
                },
                "idempotencyKey": {
                    "type": "string",
                    "description": "Optional idempotency key for the peer inbox enqueue"
                }
            },
            "required": ["to", "body", "inReplyTo"],
            "additionalProperties": false
        }),
    }
}

/// Force-include inbox_reply when allowlist is non-empty. Author: kejiqing
pub fn ensure_inbox_reply_in_allowed_tools(tools: &mut Vec<String>) {
    if tools.is_empty() {
        return;
    }
    if tools.iter().any(|t| t == INBOX_REPLY_TOOL_NAME) {
        return;
    }
    tools.push(INBOX_REPLY_TOOL_NAME.to_string());
}

fn local_from_address() -> Result<MailboxAddress, ToolError> {
    let session_id = std::env::var("CLAW_SESSION_ID")
        .map_err(|_| ToolError::new("CLAW_SESSION_ID not set"))?;
    let proj_id: i64 = std::env::var("CLAW_PROJ_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&id| id >= 1)
        .ok_or_else(|| ToolError::new("CLAW_PROJ_ID not set"))?;
    let cluster_id = std::env::var("CLAW_CLUSTER_ID")
        .map_err(|_| ToolError::new("CLAW_CLUSTER_ID not set"))?;
    MailboxAddress::new(session_id, proj_id, cluster_id).map_err(ToolError::new)
}

fn gateway_base() -> Result<String, ToolError> {
    std::env::var("CLAW_GATEWAY_BASE")
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::new("CLAW_GATEWAY_BASE not set"))
}

/// POST peer inbox with from=local address. Author: kejiqing
pub fn run_inbox_reply(input: &InboxReplyInput) -> Result<String, ToolError> {
    let body = input.body.trim();
    if body.is_empty() {
        return Err(ToolError::new("body must be non-empty"));
    }
    let in_reply_to = input
        .in_reply_to
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::new("inReplyTo is required"))?;
    let to = parse_mailbox_address(&input.to).map_err(ToolError::new)?;
    let from = local_from_address()?;
    if to.cluster_id != from.cluster_id {
        return Err(ToolError::new(format!(
            "cross-cluster reply not supported (to.cluster={}, from.cluster={})",
            to.cluster_id, from.cluster_id
        )));
    }
    let base = gateway_base()?;
    let url = format!("{}/v1/sessions/{}/inbox", base, to.session_id);
    let mut payload = json!({
        "projId": to.proj_id,
        "source": "mailbox",
        "body": body,
        "fromAddress": from.format(),
        "inReplyTo": in_reply_to,
    });
    if let Some(refs) = &input.references {
        let cleaned: Vec<String> = refs
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !cleaned.is_empty() {
            payload["references"] = json!(cleaned);
        }
    }
    if let Some(k) = input
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        payload["idempotencyKey"] = json!(k);
    }
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|e| ToolError::new(format!("http client: {e}")))?;
    let resp = client
        .post(&url)
        .json(&payload)
        .send()
        .map_err(|e| ToolError::new(format!("POST {url}: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .map_err(|e| ToolError::new(format!("read body: {e}")))?;
    if !status.is_success() {
        return Err(ToolError::new(format!("POST {url} {status}: {text}")));
    }
    let v: Value = serde_json::from_str(&text)
        .unwrap_or_else(|_| json!({ "raw": text }));
    Ok(json!({
        "ok": true,
        "to": to.format(),
        "from": from.format(),
        "peer": v,
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_schema_requires_in_reply_to() {
        let def = inbox_reply_tool_definition();
        assert_eq!(def.name, INBOX_REPLY_TOOL_NAME);
        let req = def.input_schema.get("required").unwrap().as_array().unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("inReplyTo")));
        assert!(req.iter().any(|v| v.as_str() == Some("to")));
    }

    #[test]
    fn ensure_adds_when_non_empty() {
        let mut t = vec!["bash".into()];
        ensure_inbox_reply_in_allowed_tools(&mut t);
        assert!(t.iter().any(|x| x == INBOX_REPLY_TOOL_NAME));
        ensure_inbox_reply_in_allowed_tools(&mut t);
        assert_eq!(t.iter().filter(|x| *x == INBOX_REPLY_TOOL_NAME).count(), 1);
    }
}
