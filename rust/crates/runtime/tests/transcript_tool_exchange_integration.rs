//! Integration: transcript tool_use/tool_result pairing → OpenAI-compat payload validity.
//!
//! Deterministic mock-API coverage for F16 (`tool_calls` without following tool messages).
//! Author: kejiqing

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use api::{
    build_chat_completion_request, translate_message, InputContentBlock, InputMessage,
    MessageRequest, OpenAiCompatConfig,
};
use runtime::{
    ApiClient, ApiRequest, AssistantEvent, ContentBlock, ConversationMessage,
    ConversationRuntime, MessageRole, PermissionMode, PermissionPolicy, RuntimeError,
    Session, StaticToolExecutor, ToolError,
};
use serde_json::{json, Value};

fn temp_jsonl(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!("claw-transcript-it-{label}-{nanos}.jsonl"))
}

fn count_jsonl_messages(path: &Path) -> usize {
    if !path.is_file() {
        return 0;
    }
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .is_some_and(|row| row.get("type").and_then(Value::as_str) == Some("message"))
        })
        .count()
}

fn convert_runtime_messages_to_api(messages: &[ConversationMessage]) -> Vec<InputMessage> {
    messages
        .iter()
        .map(|message| {
            let role = match message.role {
                MessageRole::System => "system",
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
                MessageRole::Tool => "user",
            }
            .to_string();
            let content = message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => {
                        Some(InputContentBlock::Text { text: text.clone() })
                    }
                    ContentBlock::ReasoningContent { text } => {
                        Some(InputContentBlock::ReasoningContent { text: text.clone() })
                    }
                    ContentBlock::ToolUse { id, name, input } => {
                        let parsed =
                            serde_json::from_str::<Value>(input).unwrap_or_else(|_| json!({}));
                        Some(InputContentBlock::ToolUse {
                            id: id.clone(),
                            name: name.clone(),
                            input: parsed,
                        })
                    }
                    ContentBlock::ToolResult {
                        tool_use_id,
                        output,
                        is_error,
                        ..
                    } => Some(InputContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: vec![api::ToolResultContentBlock::Text {
                            text: output.clone(),
                        }],
                        is_error: *is_error,
                    }),
                })
                .collect();
            InputMessage { role, content }
        })
        .collect()
}

fn openai_wire_messages(session: &Session) -> Vec<Value> {
    let request = MessageRequest {
        model: "openai/gpt-4o".to_string(),
        max_tokens: 1024,
        messages: convert_runtime_messages_to_api(&session.messages),
        stream: false,
        ..Default::default()
    };
    build_chat_completion_request(&request, OpenAiCompatConfig::openai())["messages"]
        .as_array()
        .expect("messages array")
        .clone()
}

/// Mirrors provider rule from F16: every `assistant.tool_calls[]` id must be answered by a
/// following `role:"tool"` message before the next non-tool turn.
fn assert_openai_tool_calls_fully_answered(messages: &[Value]) {
    let mut pending = HashSet::new();
    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
        match role {
            "assistant" => {
                if let Some(tool_calls) = msg.get("tool_calls").and_then(Value::as_array) {
                    for tc in tool_calls {
                        if let Some(id) = tc.get("id").and_then(Value::as_str) {
                            pending.insert(id.to_string());
                        }
                    }
                }
            }
            "tool" => {
                if let Some(id) = msg.get("tool_call_id").and_then(Value::as_str) {
                    pending.remove(id);
                }
            }
            "user" | "system" => {
                assert!(
                    pending.is_empty(),
                    "provider would 400: unanswered tool_calls {pending:?} before role={role}: {msg:?}"
                );
            }
            _ => {}
        }
    }
    assert!(
        pending.is_empty(),
        "provider would 400: dangling tool_calls at end: {pending:?}"
    );
}

fn assert_session_transcript_api_safe(session: &Session) {
    assert!(session.unanswered_tool_uses().is_empty());
    assert_openai_tool_calls_fully_answered(&openai_wire_messages(session));
}

struct CapturingApi {
    calls: Arc<Mutex<Vec<ApiRequest>>>,
    script: Vec<Vec<AssistantEvent>>,
}

impl CapturingApi {
    fn new(script: Vec<Vec<AssistantEvent>>) -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            script,
        }
    }
}

impl ApiClient for CapturingApi {
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        self.calls.lock().expect("lock").push(request);
        let idx = self.calls.lock().expect("lock").len() - 1;
        self.script
            .get(idx)
            .cloned()
            .ok_or_else(|| RuntimeError::new(format!("unexpected api call #{idx}")))
    }
}

fn tool_then_text_script(tool_id: &str, tool_name: &str) -> Vec<Vec<AssistantEvent>> {
    vec![
        vec![
            AssistantEvent::ToolUse {
                id: tool_id.to_string(),
                name: tool_name.to_string(),
                input: "{}".to_string(),
            },
            AssistantEvent::MessageStop,
        ],
        vec![
            AssistantEvent::TextDelta("done".to_string()),
            AssistantEvent::MessageStop,
        ],
    ]
}

#[test]
fn f16_repro_legacy_dangling_transcript_fails_provider_invariant() {
    let path = temp_jsonl("f16-legacy");
    let mut session = Session::new().with_persistence_path(path.clone());
    session.save_to_path(&path).expect("bootstrap");
    session
        .push_message(ConversationMessage::assistant(vec![ContentBlock::ToolUse {
            id: "call_dangling".to_string(),
            name: "WebFetch".to_string(),
            input: "{}".to_string(),
        }]))
        .expect("legacy half assistant");
    session
        .push_user_text("continue after interrupt")
        .expect("user");

    let wire = openai_wire_messages(&session);
    let err = std::panic::catch_unwind(|| assert_openai_tool_calls_fully_answered(&wire));
    assert!(
        err.is_err(),
        "legacy dangling tool_use must violate provider pairing (F16 repro)"
    );
    fs::remove_file(path).ok();
}

#[test]
fn mock_api_multi_turn_after_tool_error_yields_valid_openai_payload() {
    let path = temp_jsonl("tool-err-followup");
    let session = Session::new().with_persistence_path(path.clone());
    session.save_to_path(&path).expect("bootstrap");

    let api = CapturingApi::new(tool_then_text_script("call_err", "WebFetch"));
    let calls = Arc::clone(&api.calls);
    let mut runtime = ConversationRuntime::new(
        session,
        api,
        StaticToolExecutor::new().register("WebFetch", |_input| {
            Err(ToolError::new("context deadline exceeded"))
        }),
        PermissionPolicy::new(PermissionMode::DangerFullAccess),
        vec!["system".to_string()],
    );

    runtime
        .run_turn("fetch page", None)
        .expect("turn 1 should complete");

    let api2 = CapturingApi::new(vec![vec![
        AssistantEvent::TextDelta("continuing".to_string()),
        AssistantEvent::MessageStop,
    ]]);
    let calls2 = Arc::clone(&api2.calls);
    let session = runtime.into_session();
    let mut runtime = ConversationRuntime::new(
        session,
        api2,
        StaticToolExecutor::new(),
        PermissionPolicy::new(PermissionMode::DangerFullAccess),
        vec!["system".to_string()],
    );
    runtime
        .run_turn("next question", None)
        .expect("turn 2 should not 400 on history");

    assert_eq!(calls.lock().expect("lock").len(), 2);
    assert_eq!(calls2.lock().expect("lock").len(), 1);
    let restored = Session::load_from_path(&path).expect("reload jsonl");
    assert_session_transcript_api_safe(&restored);
    fs::remove_file(path).ok();
}

#[test]
fn mock_api_multi_turn_after_successful_tool_yields_valid_openai_payload() {
    let path = temp_jsonl("tool-ok-followup");
    let session = Session::new().with_persistence_path(path.clone());
    session.save_to_path(&path).expect("bootstrap");

    let api = CapturingApi::new(tool_then_text_script("call_ok", "WebFetch"));
    let mut runtime = ConversationRuntime::new(
        session,
        api,
        StaticToolExecutor::new().register("WebFetch", |_input| Ok("200 OK".to_string())),
        PermissionPolicy::new(PermissionMode::DangerFullAccess),
        vec!["system".to_string()],
    );

    runtime.run_turn("fetch", None).expect("turn 1");
    let session = runtime.into_session();
    let mut runtime = ConversationRuntime::new(
        session,
        CapturingApi::new(vec![vec![
            AssistantEvent::TextDelta("ok".to_string()),
            AssistantEvent::MessageStop,
        ]]),
        StaticToolExecutor::new(),
        PermissionPolicy::new(PermissionMode::DangerFullAccess),
        vec!["system".to_string()],
    );
    runtime.run_turn("follow up", None).expect("turn 2");

    let restored = Session::load_from_path(&path).expect("reload");
    assert_session_transcript_api_safe(&restored);
    fs::remove_file(path).ok();
}

#[test]
fn slow_tool_does_not_persist_half_assistant_before_exchange_completes() {
    let path = temp_jsonl("slow-tool");
    let session = Session::new().with_persistence_path(path.clone());
    session.save_to_path(&path).expect("bootstrap");
    let before = count_jsonl_messages(&path);

    let (tool_started_tx, tool_started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_unblock = release_tx.clone();

    let api = CapturingApi::new(tool_then_text_script("call_slow", "WebFetch"));
    let mut runtime = ConversationRuntime::new(
        session,
        api,
        StaticToolExecutor::new().register("WebFetch", move |_input| {
            let _ = tool_started_tx.send(());
            let _ = release_rx.recv();
            Ok("late".to_string())
        }),
        PermissionPolicy::new(PermissionMode::DangerFullAccess),
        vec!["system".to_string()],
    );

    let path_checker = path.clone();
    let checker = thread::spawn(move || {
        tool_started_rx.recv().expect("tool should start");
        let restored = Session::load_from_path(&path_checker).expect("reload mid-flight");
        assert!(
            restored.unanswered_tool_uses().is_empty(),
            "half assistant tool_use must not hit jsonl mid-dispatch"
        );
        let count = count_jsonl_messages(&path_checker);
        let _ = release_unblock.send(());
        count
    });

    let turn_result = runtime.run_turn("slow fetch", None);
    let mid_count = checker.join().expect("checker join");
    turn_result.expect("turn completes");

    assert_eq!(mid_count, before + 1, "only user row before exchange completes");

    let restored = Session::load_from_path(&path).expect("reload final");
    assert_session_transcript_api_safe(&restored);
    fs::remove_file(path).ok();
}

#[test]
fn mock_api_second_turn_request_sees_paired_history() {
    let path = temp_jsonl("second-request-history");
    let session = Session::new().with_persistence_path(path.clone());
    session.save_to_path(&path).expect("bootstrap");

    let api = CapturingApi::new(tool_then_text_script("call_hist", "bash"));
    let calls = Arc::clone(&api.calls);
    let mut runtime = ConversationRuntime::new(
        session,
        api,
        StaticToolExecutor::new().register("bash", |_input| Ok("ok".to_string())),
        PermissionPolicy::new(PermissionMode::DangerFullAccess),
        vec!["system".to_string()],
    );
    runtime.run_turn("run", None).expect("turn 1");

    let session = runtime.into_session();
    let api2 = CapturingApi::new(vec![vec![
        AssistantEvent::TextDelta("reply".to_string()),
        AssistantEvent::MessageStop,
    ]]);
    let calls2 = Arc::clone(&api2.calls);
    let mut runtime = ConversationRuntime::new(
        session,
        api2,
        StaticToolExecutor::new(),
        PermissionPolicy::new(PermissionMode::DangerFullAccess),
        vec!["system".to_string()],
    );
    runtime.run_turn("again", None).expect("turn 2");

    let second_request = calls2.lock().expect("lock")[0].clone();
    let wire: Vec<Value> = second_request
        .messages
        .iter()
        .flat_map(|message| {
            let api_msg = convert_runtime_messages_to_api(std::slice::from_ref(message));
            api_msg
                .iter()
                .flat_map(|m| translate_message(m, "gpt-4o"))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_openai_tool_calls_fully_answered(&wire);
    assert_eq!(calls.lock().expect("lock").len(), 2);
    fs::remove_file(path).ok();
}
