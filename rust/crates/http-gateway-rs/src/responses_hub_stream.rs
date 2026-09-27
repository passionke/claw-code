//! Responses `stream=true`: open SSE first, enqueue solve, pump LiveReportHub. Author: kejiqing
//!
//! Existing frames stay the same JSON. Thinking, shell output, kind, and display are extra events.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use axum::http::{header, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{AppendHeaders, IntoResponse, Response};
use futures_util::stream;
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

use crate::agent_completion::responses_api_response;
use crate::pool::{HubMsg, LiveReportHub, ProcessEvent};
use crate::session_db::{GatewaySessionDb, TurnModelUsageRow};

/// Per-kind collapsed/expanded hints from `nerogate.display`. Unknown values are ignored.
/// Author: kejiqing
#[derive(Clone, Debug, Default)]
pub struct ResponsesDisplay {
    by_kind: HashMap<String, String>,
}

impl ResponsesDisplay {
    /// `nerogate` object from the request body. Missing or invalid entries use defaults.
    #[must_use]
    pub fn parse(nerogate: Option<&Value>) -> Self {
        let mut by_kind = HashMap::new();
        let Some(obj) = nerogate
            .and_then(|v| v.get("display"))
            .and_then(Value::as_object)
        else {
            return Self { by_kind };
        };
        for (kind, mode) in obj {
            let Some(mode) = mode.as_str() else {
                continue;
            };
            if mode == "collapsed" || mode == "expanded" {
                by_kind.insert(kind.clone(), mode.to_string());
            }
        }
        Self { by_kind }
    }

    fn resolve(&self, kind: &str) -> String {
        if let Some(mode) = self.by_kind.get(kind) {
            return mode.clone();
        }
        if kind == "thinking" {
            if let Some(mode) = self.by_kind.get("reasoning") {
                return mode.clone();
            }
        }
        default_display(kind).to_string()
    }
}

fn default_display(kind: &str) -> &'static str {
    match kind {
        "shell" | "ask" => "expanded",
        _ => "collapsed",
    }
}

struct ResponsesCursor {
    turn_id: String,
    display: ResponsesDisplay,
    text_segment: String,
    reasoning_open: bool,
    reasoning_text: String,
    reasoning_seq: u32,
    /// Arguments captured at `tool.start`. `tool.end` does not repeat them. Author: kejiqing
    tool_args: HashMap<String, String>,
}

impl ResponsesCursor {
    fn new(turn_id: String, display: ResponsesDisplay) -> Self {
        Self {
            turn_id,
            display,
            text_segment: String::new(),
            reasoning_open: false,
            reasoning_text: String::new(),
            reasoning_seq: 0,
            tool_args: HashMap::new(),
        }
    }
}

fn nerogate(kind: &str, display: &str) -> Value {
    json!({ "kind": kind, "display": display })
}

fn tool_id(pe: &ProcessEvent) -> String {
    pe.payload
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn tool_kind(pe: &ProcessEvent) -> String {
    pe.payload
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_string()
}

fn args_summary(pe: &ProcessEvent) -> String {
    pe.payload
        .get("argsSummary")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn reasoning_item_id(cursor: &ResponsesCursor) -> String {
    format!("rs_{}_{}", cursor.turn_id, cursor.reasoning_seq)
}

fn close_text(cursor: &mut ResponsesCursor, out: &mut Vec<(String, Value)>) {
    if cursor.text_segment.is_empty() {
        return;
    }
    out.push((
        "response.output_text.done".into(),
        json!({
            "type": "response.output_text.done",
            "text": cursor.text_segment,
        }),
    ));
    cursor.text_segment.clear();
}

fn close_reasoning(cursor: &mut ResponsesCursor, out: &mut Vec<(String, Value)>) {
    if !cursor.reasoning_open {
        return;
    }
    let id = reasoning_item_id(cursor);
    let display = cursor.display.resolve("thinking");
    out.push((
        "response.reasoning_text.done".into(),
        json!({
            "type": "response.reasoning_text.done",
            "item_id": id,
            "text": cursor.reasoning_text,
            "nerogate": nerogate("thinking", &display),
        }),
    ));
    out.push((
        "response.output_item.done".into(),
        json!({
            "type": "response.output_item.done",
            "item": { "type": "reasoning", "id": id },
        }),
    ));
    cursor.reasoning_open = false;
    cursor.reasoning_text.clear();
    cursor.reasoning_seq = cursor.reasoning_seq.saturating_add(1);
}

fn legacy_tool_added(pe: &ProcessEvent) -> Value {
    json!({
        "type": "response.output_item.added",
        "item": {
            "type": "function_call",
            "id": pe.payload.get("toolCallId"),
            "name": pe.payload.get("name"),
            "arguments": pe.payload.get("argsSummary").and_then(|v| v.as_str()).unwrap_or(""),
        }
    })
}

/// Project one hub message into SSE frames. Legacy `output_text.delta` and
/// `output_item.added` JSON stay unchanged. Author: kejiqing
fn project_responses_msg(
    cursor: &mut ResponsesCursor,
    msg: &HubMsg,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    match msg {
        HubMsg::Delta(d) => {
            if d.text.is_empty() {
                return out;
            }
            close_reasoning(cursor, &mut out);
            cursor.text_segment.push_str(&d.text);
            out.push((
                "response.output_text.delta".into(),
                json!({
                    "type": "response.output_text.delta",
                    "delta": d.text,
                }),
            ));
        }
        HubMsg::Process(pe) if pe.ev == "thinking.delta" => {
            let text = pe
                .payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("");
            if text.is_empty() {
                return out;
            }
            close_text(cursor, &mut out);
            let display = cursor.display.resolve("thinking");
            if !cursor.reasoning_open {
                let id = reasoning_item_id(cursor);
                out.push((
                    "response.output_item.added".into(),
                    json!({
                        "type": "response.output_item.added",
                        "item": {
                            "type": "reasoning",
                            "id": id,
                        },
                        "nerogate": nerogate("thinking", &display),
                    }),
                ));
                cursor.reasoning_open = true;
            }
            cursor.reasoning_text.push_str(text);
            let id = reasoning_item_id(cursor);
            out.push((
                "response.reasoning_text.delta".into(),
                json!({
                    "type": "response.reasoning_text.delta",
                    "item_id": id,
                    "delta": text,
                    "nerogate": nerogate("thinking", &display),
                }),
            ));
        }
        HubMsg::Process(pe) if pe.ev == "tool.start" => {
            close_text(cursor, &mut out);
            close_reasoning(cursor, &mut out);
            out.push((
                "response.output_item.added".into(),
                legacy_tool_added(pe),
            ));
            let id = tool_id(pe);
            let kind = tool_kind(pe);
            let display = cursor.display.resolve(&kind);
            let args = args_summary(pe);
            cursor.tool_args.insert(id.clone(), args.clone());
            if kind == "mcp" {
                out.push((
                    "response.mcp_call.in_progress".into(),
                    json!({
                        "type": "response.mcp_call.in_progress",
                        "item_id": id,
                        "nerogate": nerogate("mcp", &display),
                    }),
                ));
                out.push((
                    "response.mcp_call_arguments.delta".into(),
                    json!({
                        "type": "response.mcp_call_arguments.delta",
                        "item_id": id,
                        "delta": args,
                        "nerogate": nerogate("mcp", &display),
                    }),
                ));
            } else {
                out.push((
                    "response.function_call_arguments.delta".into(),
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": id,
                        "delta": args,
                        "nerogate": nerogate(&kind, &display),
                    }),
                ));
            }
        }
        HubMsg::Process(pe) if pe.ev == "tool.end" => {
            let id = tool_id(pe);
            let kind = tool_kind(pe);
            let display = cursor.display.resolve(&kind);
            let args = cursor
                .tool_args
                .remove(&id)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| args_summary(pe));
            if kind == "mcp" {
                let status = pe
                    .payload
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("ok");
                let event = if status == "ok" {
                    "response.mcp_call.completed"
                } else {
                    "response.mcp_call.failed"
                };
                out.push((
                    event.into(),
                    json!({
                        "type": event,
                        "item_id": id,
                        "nerogate": nerogate("mcp", &display),
                    }),
                ));
            } else {
                out.push((
                    "response.function_call_arguments.done".into(),
                    json!({
                        "type": "response.function_call_arguments.done",
                        "item_id": id,
                        "arguments": args,
                        "nerogate": nerogate(&kind, &display),
                    }),
                ));
            }
        }
        HubMsg::Process(pe) if pe.ev == "shell.chunk" => {
            let id = tool_id(pe);
            let text = pe
                .payload
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or("");
            if id.is_empty() || text.is_empty() {
                return out;
            }
            out.push((
                "response.nerogate.shell_output.delta".into(),
                json!({
                    "type": "response.nerogate.shell_output.delta",
                    "item_id": id,
                    "delta": text,
                }),
            ));
        }
        HubMsg::AskUser(pending) => {
            close_text(cursor, &mut out);
            close_reasoning(cursor, &mut out);
            let display = cursor.display.resolve("ask");
            out.push((
                "response.nerogate.ask".into(),
                json!({
                    "type": "response.nerogate.ask",
                    "questionId": pending.question_id,
                    "question": pending.question,
                    "options": pending.options,
                    "nerogate": nerogate("ask", &display),
                }),
            ));
        }
        HubMsg::Process(_) | HubMsg::AskUserCleared | HubMsg::SolveDone => {
            if matches!(msg, HubMsg::SolveDone) {
                close_text(cursor, &mut out);
                close_reasoning(cursor, &mut out);
            }
        }
    }
    out
}

/// SSE from Hub for OpenAI Responses stream. Author: kejiqing
pub fn responses_hub_sse_response(
    hub: Arc<LiveReportHub>,
    model: String,
    session_id: String,
    turn_id: String,
    session_db: Arc<GatewaySessionDb>,
    display: ResponsesDisplay,
) -> Response {
    let (tx, rx) = mpsc::unbounded_channel::<(String, String)>();
    let hub_done = Arc::clone(&hub);
    tokio::spawn(async move {
        let created_at = chrono::Utc::now().timestamp();
        let send = |tx: &mpsc::UnboundedSender<(String, String)>, event: String, data: Value| {
            tx.send((event, data.to_string())).is_ok()
        };
        if !send(
            &tx,
            "response.created".into(),
            json!({
                "type": "response.created",
                "response": {
                    "id": turn_id,
                    "object": "response",
                    "created_at": created_at,
                    "status": "in_progress",
                    "model": model,
                    "output": []
                }
            }),
        ) {
            return;
        }
        let _ = send(
            &tx,
            "response.in_progress".into(),
            json!({
                "type": "response.in_progress",
                "response": { "id": turn_id, "status": "in_progress" }
            }),
        );

        let (mut sub, snapshot, process_snapshot, pending_ask) =
            hub.subscribe_with_process_snapshot(&turn_id);
        let mut cursor = ResponsesCursor::new(turn_id.clone(), display);
        let mut acc = String::new();
        for chunk in snapshot {
            if chunk.text.is_empty() {
                continue;
            }
            acc.push_str(&chunk.text);
            for (event, data) in project_responses_msg(&mut cursor, &HubMsg::Delta(chunk)) {
                if !send(&tx, event, data) {
                    return;
                }
            }
        }
        for pe in process_snapshot {
            let msg = HubMsg::Process(pe);
            for (event, data) in project_responses_msg(&mut cursor, &msg) {
                if !send(&tx, event, data) {
                    return;
                }
            }
        }
        if let Some(ask) = pending_ask {
            for (event, data) in project_responses_msg(&mut cursor, &HubMsg::AskUser(ask)) {
                if !send(&tx, event, data) {
                    return;
                }
            }
        }

        let mut done = hub.is_solve_done(&turn_id);
        if !done {
            loop {
                match sub.recv().await {
                    Ok(msg) => {
                        if let HubMsg::Delta(d) = &msg {
                            acc.push_str(&d.text);
                        }
                        let finished = matches!(msg, HubMsg::SolveDone);
                        for (event, data) in project_responses_msg(&mut cursor, &msg) {
                            if !send(&tx, event, data) {
                                return;
                            }
                        }
                        if finished {
                            done = true;
                            break;
                        }
                    }
                    Err(RecvError::Closed) => {
                        done = true;
                        break;
                    }
                    Err(RecvError::Lagged(_)) => {
                        if hub.is_solve_done(&turn_id) {
                            done = true;
                            break;
                        }
                    }
                }
            }
        } else {
            for (event, data) in project_responses_msg(&mut cursor, &HubMsg::SolveDone) {
                let _ = send(&tx, event, data);
            }
        }

        if acc.is_empty() {
            acc = hub.snapshot_text(&turn_id);
        }
        let usage_rows: Vec<TurnModelUsageRow> = session_db
            .list_model_usage_for_turn(&turn_id)
            .await
            .unwrap_or_default();
        let completed = responses_api_response(
            &model,
            &turn_id,
            &session_id,
            &acc,
            created_at * 1000,
            &usage_rows,
        );
        let _ = tx.send(("response.completed".into(), completed.to_string()));
        let _ = tx.send(("done".into(), "[DONE]".into()));
        if done {
            hub_done.try_remove_turn(&turn_id);
        }
    });

    let event_stream = stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some((event, data)) => {
                let ev = Ok::<Event, Infallible>(Event::default().event(event).data(data));
                Some((ev, rx))
            }
            None => None,
        }
    });
    let no_buffer = header::HeaderName::from_static("x-accel-buffering");
    (
        AppendHeaders([(no_buffer, HeaderValue::from_static("no"))]),
        Sse::new(event_stream).keep_alive(KeepAlive::default()),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::{HubDeltaChunk, ProcessEvent};

    fn cursor() -> ResponsesCursor {
        ResponsesCursor::new("T1".into(), ResponsesDisplay::default())
    }

    fn tool_start(name: &str, kind: &str, args: &str) -> HubMsg {
        HubMsg::Process(ProcessEvent {
            ev: "tool.start".into(),
            payload: json!({
                "ev": "tool.start",
                "toolCallId": "tc1",
                "name": name,
                "kind": kind,
                "argsSummary": args,
            }),
        })
    }

    #[test]
    fn text_delta_json_is_unchanged() {
        let mut c = cursor();
        let frames = project_responses_msg(
            &mut c,
            &HubMsg::Delta(HubDeltaChunk {
                text: "hi".into(),
                emit_seq: None,
            }),
        );
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, "response.output_text.delta");
        assert_eq!(
            frames[0].1,
            json!({"type": "response.output_text.delta", "delta": "hi"})
        );
    }

    #[test]
    fn tool_start_keeps_legacy_output_item_and_adds_arguments_delta() {
        let mut c = cursor();
        let frames = project_responses_msg(&mut c, &tool_start("Grep", "search", "foo"));
        let added = frames
            .iter()
            .find(|(name, _)| name == "response.output_item.added")
            .expect("legacy item");
        assert_eq!(
            added.1,
            json!({
                "type": "response.output_item.added",
                "item": {
                    "type": "function_call",
                    "id": "tc1",
                    "name": "Grep",
                    "arguments": "foo"
                }
            })
        );
        assert!(added.1.get("nerogate").is_none());
        let delta = frames
            .iter()
            .find(|(name, _)| name == "response.function_call_arguments.delta")
            .expect("arguments delta");
        assert_eq!(delta.1["item_id"], "tc1");
        assert_eq!(delta.1["delta"], "foo");
        assert_eq!(delta.1["nerogate"]["kind"], "search");
        assert_eq!(delta.1["nerogate"]["display"], "collapsed");
    }

    #[test]
    fn shell_chunks_share_the_function_call_id() {
        let mut c = cursor();
        let _ = project_responses_msg(&mut c, &tool_start("Bash", "shell", "ls"));
        let chunk = HubMsg::Process(ProcessEvent {
            ev: "shell.chunk".into(),
            payload: json!({"toolCallId": "tc1", "text": "a\n"}),
        });
        let again = HubMsg::Process(ProcessEvent {
            ev: "shell.chunk".into(),
            payload: json!({"toolCallId": "tc1", "text": "b\n"}),
        });
        let first = project_responses_msg(&mut c, &chunk);
        let second = project_responses_msg(&mut c, &again);
        assert_eq!(first[0].0, "response.nerogate.shell_output.delta");
        assert_eq!(first[0].1["item_id"], "tc1");
        assert_eq!(second[0].1["item_id"], "tc1");
        assert_eq!(second[0].1["delta"], "b\n");
    }

    #[test]
    fn shell_display_defaults_to_expanded() {
        let mut c = cursor();
        let frames = project_responses_msg(&mut c, &tool_start("Bash", "shell", "ls"));
        let delta = frames
            .iter()
            .find(|(name, _)| name == "response.function_call_arguments.delta")
            .expect("delta");
        assert_eq!(delta.1["nerogate"]["display"], "expanded");
    }

    #[test]
    fn invalid_display_falls_back() {
        let display = ResponsesDisplay::parse(Some(&json!({
            "display": { "search": "sideways", "shell": "collapsed" }
        })));
        assert_eq!(display.resolve("search"), "collapsed");
        assert_eq!(display.resolve("shell"), "collapsed");
        assert_eq!(display.resolve("ask"), "expanded");
    }

    #[test]
    fn completed_output_stays_a_single_message() {
        let body = responses_api_response("agent", "T1", "sess", "hello", 1_000, &[]);
        let output = body["output"].as_array().expect("output");
        assert_eq!(output.len(), 1);
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["content"][0]["type"], "output_text");
        assert_eq!(output[0]["content"][0]["text"], "hello");
    }

    #[test]
    fn thinking_then_text_closes_reasoning_without_touching_text_delta() {
        let mut c = cursor();
        let open = project_responses_msg(
            &mut c,
            &HubMsg::Process(ProcessEvent {
                ev: "thinking.delta".into(),
                payload: json!({"ev": "thinking.delta", "text": "先"}),
            }),
        );
        assert_eq!(open[0].0, "response.output_item.added");
        assert_eq!(open[0].1["item"]["type"], "reasoning");
        assert_eq!(open[0].1["nerogate"]["display"], "collapsed");
        assert_eq!(open[1].0, "response.reasoning_text.delta");
        assert_eq!(open[1].1["delta"], "先");

        let more = project_responses_msg(
            &mut c,
            &HubMsg::Process(ProcessEvent {
                ev: "thinking.delta".into(),
                payload: json!({"text": "想"}),
            }),
        );
        assert_eq!(more.len(), 1);
        assert_eq!(more[0].0, "response.reasoning_text.delta");

        let text = project_responses_msg(
            &mut c,
            &HubMsg::Delta(HubDeltaChunk {
                text: "正文".into(),
                emit_seq: None,
            }),
        );
        assert_eq!(text[0].0, "response.reasoning_text.done");
        assert_eq!(text[0].1["text"], "先想");
        assert_eq!(text[1].0, "response.output_item.done");
        assert_eq!(text[1].1["item"]["type"], "reasoning");
        assert_eq!(text[2].0, "response.output_text.delta");
        assert_eq!(
            text[2].1,
            json!({"type": "response.output_text.delta", "delta": "正文"})
        );
    }

    #[test]
    fn tool_after_text_closes_the_text_segment_and_keeps_start_arguments() {
        let mut c = cursor();
        let _ = project_responses_msg(
            &mut c,
            &HubMsg::Delta(HubDeltaChunk {
                text: "先说".into(),
                emit_seq: None,
            }),
        );
        let start = project_responses_msg(&mut c, &tool_start("Grep", "search", "订单"));
        assert_eq!(start[0].0, "response.output_text.done");
        assert_eq!(start[0].1["text"], "先说");
        assert_eq!(start[1].0, "response.output_item.added");
        assert!(start[1].1.get("nerogate").is_none());

        let end = project_responses_msg(
            &mut c,
            &HubMsg::Process(ProcessEvent {
                ev: "tool.end".into(),
                payload: json!({
                    "toolCallId": "tc1",
                    "kind": "search",
                    "status": "ok",
                    "resultSummary": "12 hits"
                }),
            }),
        );
        assert_eq!(end[0].0, "response.function_call_arguments.done");
        assert_eq!(end[0].1["arguments"], "订单");
        assert_eq!(end[0].1["nerogate"]["kind"], "search");
    }

    #[test]
    fn mcp_uses_mcp_events_and_keeps_the_legacy_function_call_item() {
        let mut c = cursor();
        let start = project_responses_msg(&mut c, &tool_start("sqlbot", "mcp", "问销售"));
        let names: Vec<_> = start.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "response.output_item.added",
                "response.mcp_call.in_progress",
                "response.mcp_call_arguments.delta",
            ]
        );
        assert!(start
            .iter()
            .all(|(name, body)| name != "response.output_item.added" || body.get("nerogate").is_none()));
        let end = project_responses_msg(
            &mut c,
            &HubMsg::Process(ProcessEvent {
                ev: "tool.end".into(),
                payload: json!({"toolCallId": "tc1", "kind": "mcp", "status": "error"}),
            }),
        );
        assert_eq!(end[0].0, "response.mcp_call.failed");
        assert_eq!(end[0].1["item_id"], "tc1");
    }

    #[test]
    fn ask_is_its_own_event_and_defaults_to_expanded() {
        let mut c = cursor();
        let frames = project_responses_msg(
            &mut c,
            &HubMsg::AskUser(crate::pool::AskUserPending {
                question_id: "aq1".into(),
                question: "选哪个？".into(),
                options: Some(vec!["A".into(), "B".into()]),
                a2ui: json!({}),
            }),
        );
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, "response.nerogate.ask");
        assert_eq!(frames[0].1["questionId"], "aq1");
        assert_eq!(frames[0].1["nerogate"]["kind"], "ask");
        assert_eq!(frames[0].1["nerogate"]["display"], "expanded");
        assert!(frames[0].1.get("delta").is_none());
    }

    #[test]
    fn request_display_overrides_the_default() {
        let display = ResponsesDisplay::parse(Some(&json!({
            "display": { "search": "expanded", "shell": "collapsed" }
        })));
        let mut c = ResponsesCursor::new("T1".into(), display);
        let frames = project_responses_msg(&mut c, &tool_start("Grep", "search", "q"));
        let delta = frames
            .iter()
            .find(|(name, _)| name == "response.function_call_arguments.delta")
            .expect("delta");
        assert_eq!(delta.1["nerogate"]["display"], "expanded");
    }

    #[tokio::test]
    async fn stdout_thinking_signal_stays_out_of_report_text() {
        let body = gateway_solve_turn::gateway_stdout::thinking_delta_event("先想").expect("event");
        let line = format!(
            "{}{body}",
            gateway_solve_turn::GATEWAY_STDOUT_LINE_PREFIX
        );
        let hub = LiveReportHub::default();
        hub.ingest_stdout_line("Tsig", &line);
        assert!(hub.snapshot_text("Tsig").is_empty());
        let (_, _, replay, _) = hub.subscribe_with_process_snapshot("Tsig");
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].ev, "thinking.delta");
        let mut c = cursor();
        let frames = project_responses_msg(&mut c, &HubMsg::Process(replay[0].clone()));
        assert_eq!(frames[1].0, "response.reasoning_text.delta");
        assert_eq!(frames[1].1["delta"], "先想");
    }

    #[tokio::test]
    async fn stdout_shell_chunk_is_live_and_updates_the_same_item() {
        let hub = LiveReportHub::default();
        let (mut rx, _, _, _) = hub.subscribe_with_process_snapshot("Tsh");
        let body = gateway_solve_turn::gateway_stdout::shell_chunk_event("tc_bash", "a\n").expect("shell");
        let line = format!(
            "{}{body}",
            gateway_solve_turn::GATEWAY_STDOUT_LINE_PREFIX
        );
        hub.ingest_stdout_line("Tsh", &line);
        assert!(hub.snapshot_text("Tsh").is_empty());
        let pe = match rx.recv().await {
            Ok(HubMsg::Process(pe)) => pe,
            other => panic!("expected process, got {other:?}"),
        };
        assert_eq!(pe.ev, "shell.chunk");
        let mut c = cursor();
        let frames = project_responses_msg(&mut c, &HubMsg::Process(pe));
        assert_eq!(frames[0].0, "response.nerogate.shell_output.delta");
        assert_eq!(frames[0].1["item_id"], "tc_bash");
        assert_eq!(frames[0].1["delta"], "a\n");
        let (_, _, replay, _) = hub.subscribe_with_process_snapshot("Tsh");
        assert!(replay.iter().all(|event| event.ev != "shell.chunk"));
    }
}
