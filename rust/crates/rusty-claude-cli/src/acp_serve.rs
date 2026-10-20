//! Minimal ACP JSON-RPC agent over stdio (`claw acp serve`). Author: kejiqing
//!
//! Intentionally hand-rolled (no agent_client_protocol crate) so ACP serde features
//! do not unify into the claw workspace. Enough for initialize → session/new →
//! session/prompt one-turn editor smoke; gateway solve stays on gateway-solve-once.

use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};

static SESSION_SEQ: AtomicU64 = AtomicU64::new(1);

pub fn run_acp_serve() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut sessions = std::collections::HashMap::<String, String>::new();

    for line in stdin.lock().lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("acp-serve: invalid json: {e}");
                continue;
            }
        };
        // Notifications have no id; ignore client notifications for v1.
        if msg.get("id").is_none() {
            continue;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default();
        let params = msg.get("params").cloned().unwrap_or(json!({}));

        let result = match method {
            "initialize" => Ok(initialize_result()),
            "session/new" => {
                let cwd = params
                    .get("cwd")
                    .and_then(|c| c.as_str())
                    .unwrap_or(".")
                    .to_string();
                let session_id = format!("claw-{}", SESSION_SEQ.fetch_add(1, Ordering::Relaxed));
                sessions.insert(session_id.clone(), cwd);
                Ok(json!({ "sessionId": session_id }))
            }
            "session/load" | "session/resume" => {
                let session_id = params
                    .get("sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                if session_id.is_empty() {
                    Err((
                        String::from("InvalidParams"),
                        String::from("sessionId required"),
                    ))
                } else {
                    sessions
                        .entry(session_id.clone())
                        .or_insert_with(|| ".".into());
                    Ok(json!({ "sessionId": session_id }))
                }
            }
            "session/prompt" => {
                let session_id = params
                    .get("sessionId")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string();
                if session_id.is_empty() || !sessions.contains_key(&session_id) {
                    Err((
                        String::from("InvalidParams"),
                        String::from("unknown sessionId; call session/new first"),
                    ))
                } else {
                    let prompt_text = extract_prompt_text(params.get("prompt"));
                    let reply = format!(
                        "claw acp serve received prompt ({} chars). Full agent turns still use `claw gateway-solve-once` / REPL; this ACP surface is the editor launch contract (ROADMAP #76).",
                        prompt_text.chars().count()
                    );
                    write_notification(
                        &mut stdout,
                        json!({
                            "jsonrpc": "2.0",
                            "method": "session/update",
                            "params": {
                                "sessionId": session_id,
                                "update": {
                                    "sessionUpdate": "agent_message_chunk",
                                    "content": { "type": "text", "text": reply }
                                }
                            }
                        }),
                    )?;
                    Ok(json!({
                        "stopReason": "end_turn",
                        "usage": {
                            "totalTokens": 0,
                            "inputTokens": 0,
                            "cachedReadTokens": 0,
                            "outputTokens": 0,
                            "thoughtTokens": 0
                        }
                    }))
                }
            }
            "session/cancel" => Ok(json!({})),
            other => Err((
                String::from("MethodNotFound"),
                format!("unsupported ACP method: {other}"),
            )),
        };

        match result {
            Ok(value) => write_response(&mut stdout, id, Ok(value))?,
            Err((code, message)) => write_response(
                &mut stdout,
                id,
                Err(json!({
                    "code": -32601,
                    "message": format!("{code}: {message}")
                })),
            )?,
        }
    }
    Ok(())
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": 1,
        "agentInfo": {
            "name": "claw",
            "title": "Claw Code",
            "version": env!("CARGO_PKG_VERSION")
        },
        "agentCapabilities": {
            "loadSession": true,
            "promptCapabilities": {
                "embeddedContext": true,
                "image": false
            },
            "mcpCapabilities": {
                "http": false,
                "sse": false
            }
        },
        "authMethods": []
    })
}

fn extract_prompt_text(prompt: Option<&Value>) -> String {
    let Some(Value::Array(blocks)) = prompt else {
        return String::new();
    };
    let mut out = String::new();
    for block in blocks {
        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    }
    out
}

fn write_response(
    stdout: &mut impl Write,
    id: Value,
    result: Result<Value, Value>,
) -> io::Result<()> {
    let mut msg = json!({ "jsonrpc": "2.0", "id": id });
    match result {
        Ok(value) => {
            msg["result"] = value;
        }
        Err(err) => {
            msg["error"] = err;
        }
    }
    write_notification(stdout, msg)
}

fn write_notification(stdout: &mut impl Write, msg: Value) -> io::Result<()> {
    writeln!(stdout, "{}", serde_json::to_string(&msg)?)?;
    stdout.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_blocks() {
        let prompt = json!([
            { "type": "text", "text": "hello" },
            { "type": "text", "text": "world" }
        ]);
        assert_eq!(extract_prompt_text(Some(&prompt)), "hello\nworld");
    }

    #[test]
    fn initialize_declares_protocol_v1() {
        let v = initialize_result();
        assert_eq!(v["protocolVersion"], 1);
        assert_eq!(v["agentInfo"]["name"], "claw");
    }
}
