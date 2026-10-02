//! Replay recorded ACP streams (spike fixtures) through `TurnMapper`. Author: kejiqing

use agent_client_protocol::schema::v1::{PromptResponse, SessionNotification};
use neuro_harness::mapper::{token_usage, OutEvent, TurnMapper};
use runtime::{ContentBlock, MessageRole};
use serde_json::Value;

struct Recorded {
    updates: Vec<SessionNotification>,
    response: PromptResponse,
}

/// Updates between our `session/prompt` request and its response, plus that response.
fn load(rel: &str) -> Recorded {
    let path = format!("{}/tests/fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path).unwrap();
    let mut prompt_id = None;
    let mut updates = Vec::new();
    for line in raw.lines() {
        let rec: Value = serde_json::from_str(line).unwrap();
        let msg = &rec["msg"];
        match rec["dir"].as_str() {
            Some("out") if msg["method"] == "session/prompt" => prompt_id = Some(msg["id"].clone()),
            Some("in") if prompt_id.is_some() => {
                if msg["method"] == "session/update" {
                    updates.push(serde_json::from_value(msg["params"].clone()).unwrap());
                } else if Some(&msg["id"]) == prompt_id.as_ref() && msg.get("result").is_some() {
                    return Recorded {
                        updates,
                        response: serde_json::from_value(msg["result"].clone()).unwrap(),
                    };
                }
            }
            _ => {}
        }
    }
    panic!("{rel}: no session/prompt response");
}

fn replay(rel: &str, servers: &[&str]) -> (Vec<OutEvent>, neuro_harness::mapper::TurnTranscript) {
    let rec = load(rel);
    let mut mapper = TurnMapper::new(servers.iter().map(|s| (*s).to_string()).collect(), false);
    let mut events = Vec::new();
    for n in &rec.updates {
        events.extend(mapper.on_update(&n.update));
    }
    let (transcript, tail) = mapper.finish(rec.response.usage.as_ref().map(token_usage));
    events.extend(tail);
    (events, transcript)
}

fn starts(events: &[OutEvent]) -> Vec<(&str, &str, &str)> {
    events
        .iter()
        .filter_map(|e| match e {
            OutEvent::ToolStart {
                name, kind, input, ..
            } => Some((name.as_str(), *kind, input.as_str())),
            _ => None,
        })
        .collect()
}

fn ends(events: &[OutEvent]) -> Vec<(&str, bool, &str)> {
    events
        .iter()
        .filter_map(|e| match e {
            OutEvent::ToolEnd {
                name, ok, output, ..
            } => Some((name.as_str(), *ok, output.as_str())),
            _ => None,
        })
        .collect()
}

#[test]
fn opencode_plain_turn_is_report_text_only() {
    let (events, t) = replay("opencode/turn1_new.ndjson", &["probe"]);
    assert!(starts(&events).is_empty());
    assert!(t.message.starts_with("MOCK_REPLY"), "{}", t.message);
    assert_eq!(t.messages.len(), 1);
    assert_eq!(t.messages[0].role, MessageRole::Assistant);
    assert!(t.messages[0].usage.is_some());
}

#[test]
fn opencode_mcp_tool_turn() {
    let (events, t) = replay("opencode/turn2_resume_tool.ndjson", &["probe"]);
    // pending tool_call with empty rawInput does not start; in_progress with input does.
    assert_eq!(
        starts(&events),
        vec![("mcp__probe__echo_meta", "mcp", r#"{"note":"spike"}"#)]
    );
    assert_eq!(
        ends(&events),
        vec![("mcp__probe__echo_meta", true, "meta_keys=progressToken")]
    );
    assert_eq!(t.message, "TOOL_RESULT_SEEN: meta_keys=progressToken");
    assert_eq!(t.tool_calls, 1);

    let roles: Vec<_> = t.messages.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        vec![
            MessageRole::Assistant,
            MessageRole::Tool,
            MessageRole::Assistant
        ]
    );
    assert!(matches!(
        &t.messages[0].blocks[..],
        [ContentBlock::ToolUse { name, input, .. }] if name == "mcp__probe__echo_meta" && input == r#"{"note":"spike"}"#
    ));
    assert!(matches!(
        &t.messages[1].blocks[..],
        [ContentBlock::ToolResult { output, is_error: false, .. }] if output == "meta_keys=progressToken"
    ));
    let usage = t.messages[2].usage.expect("usage on last assistant");
    assert_eq!((usage.input_tokens, usage.output_tokens), (11, 7));
    assert!(t.messages[0].usage.is_none());
}

#[test]
fn appserver_mcp_tool_turn_uses_meta_flag_and_raw_output() {
    let (events, t) = replay("appserver/turn3_resume_tool.ndjson", &["probe"]);
    // codex reports MCP calls as `execute`; `_meta.is_mcp_tool_call` wins.
    let s = starts(&events);
    assert_eq!(s.len(), 1);
    assert_eq!((s[0].0, s[0].1), ("mcp__probe__echo_meta", "mcp"));
    let e = ends(&events);
    assert_eq!(e.len(), 1);
    assert!(e[0].1);
    assert!(e[0].2.starts_with("meta_keys=callId"), "{}", e[0].2);
    assert!(t.message.contains("TOOL_RESULT_SEEN"));
    assert_eq!(t.tool_calls, 1);
    assert!(!events
        .iter()
        .any(|e| matches!(e, OutEvent::ShellChunk { .. })));
}

#[test]
fn appserver_plain_turn() {
    let (events, t) = replay("appserver/turn1_new.ndjson", &[]);
    assert!(starts(&events).is_empty());
    assert!(t.message.contains("MOCK_REPLY"), "{}", t.message);
    let reports: String = events
        .iter()
        .filter_map(|e| match e {
            OutEvent::ReportDelta(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reports, t.message);
}
