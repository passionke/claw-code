//! ACP `session/update` → gateway stdout events + claw-format transcript messages (pure, no IO).
//! Mapping table: `docs/neuro-harness-contract.md` §3.1, §4. Author: kejiqing

use std::collections::HashMap;
use std::time::Instant;

use agent_client_protocol::schema::v1::{
    ContentBlock as AcpBlock, SessionUpdate, ToolCall, ToolCallContent, ToolCallStatus,
    ToolCallUpdate, ToolKind, Usage,
};
use runtime::{ContentBlock, ConversationMessage, TokenUsage};
use serde_json::Value;

/// One contract event derived from an ACP update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutEvent {
    ReportDelta(String),
    ThinkingDelta(String),
    ToolStart {
        id: String,
        name: String,
        kind: &'static str,
        input: String,
    },
    ShellChunk {
        id: String,
        text: String,
    },
    ToolEnd {
        id: String,
        name: String,
        kind: &'static str,
        ok: bool,
        duration_ms: u64,
        output: String,
    },
}

/// What one turn adds to the session transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnTranscript {
    /// Concatenated `agent_message_chunk` text (`outputJson.message`).
    pub message: String,
    /// Assistant / tool messages in order; usage sits on the last assistant message.
    pub messages: Vec<ConversationMessage>,
    pub tool_calls: usize,
}

#[derive(Debug)]
struct ToolState {
    name: String,
    title: String,
    kind: &'static str,
    acp_kind: ToolKind,
    is_mcp_meta: bool,
    input: Option<Value>,
    started_at: Option<Instant>,
    ended: bool,
}

#[derive(Debug)]
pub struct TurnMapper {
    mcp_servers: Vec<String>,
    thinking: bool,
    message: String,
    pending: Vec<ContentBlock>,
    messages: Vec<ConversationMessage>,
    tools: HashMap<String, ToolState>,
    tool_calls: usize,
}

impl TurnMapper {
    /// `mcp_servers`: names injected via mcp-proxy (MCP kind detection).
    /// `thinking`: emit `thinking.delta` (`responsesStream`).
    #[must_use]
    pub fn new(mcp_servers: Vec<String>, thinking: bool) -> Self {
        Self {
            mcp_servers,
            thinking,
            message: String::new(),
            pending: Vec::new(),
            messages: Vec::new(),
            tools: HashMap::new(),
            tool_calls: 0,
        }
    }

    #[must_use]
    pub fn tool_calls(&self) -> usize {
        self.tool_calls
    }

    pub fn on_update(&mut self, update: &SessionUpdate) -> Vec<OutEvent> {
        match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                let text = block_text(&chunk.content);
                if text.is_empty() {
                    return Vec::new();
                }
                self.message.push_str(&text);
                self.push_text(&text);
                vec![OutEvent::ReportDelta(text)]
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                let text = block_text(&chunk.content);
                if self.thinking && !text.is_empty() {
                    vec![OutEvent::ThinkingDelta(text)]
                } else {
                    Vec::new()
                }
            }
            SessionUpdate::ToolCall(call) => self.on_tool_call(call),
            SessionUpdate::ToolCallUpdate(update) => self.on_tool_call_update(update),
            _ => Vec::new(),
        }
    }

    /// Close the turn. Tools that never finished get an error `tool.end` / `tool_result`.
    #[must_use]
    pub fn finish(mut self, usage: Option<TokenUsage>) -> (TurnTranscript, Vec<OutEvent>) {
        let mut open: Vec<String> = self
            .tools
            .iter()
            .filter(|(_, t)| !t.ended)
            .map(|(id, _)| id.clone())
            .collect();
        open.sort();
        let mut events = Vec::new();
        for id in open {
            events.extend(self.end_tool(&id, false, "tool did not complete in this turn".into()));
        }
        self.flush_assistant();
        if let Some(last) = self
            .messages
            .iter_mut()
            .rev()
            .find(|m| m.role == runtime::MessageRole::Assistant)
        {
            last.usage = usage;
        }
        (
            TurnTranscript {
                message: self.message,
                messages: self.messages,
                tool_calls: self.tool_calls,
            },
            events,
        )
    }

    fn on_tool_call(&mut self, call: &ToolCall) -> Vec<OutEvent> {
        let id = call.tool_call_id.0.to_string();
        if self.tools.contains_key(&id) {
            return self.apply_tool_fields(
                &id,
                Some(call.kind),
                Some(call.status),
                Some(call.title.clone()),
                call.name.clone(),
                Some(&call.content),
                call.raw_input.clone(),
                call.raw_output.as_ref(),
            );
        }
        self.tool_calls += 1;
        let name = call
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| call.title.clone());
        let is_mcp_meta = meta_flag(call.meta.as_ref(), "is_mcp_tool_call");
        let kind = self.contract_kind(call.kind, &name, &call.title, is_mcp_meta);
        self.pending.push(ContentBlock::ToolUse {
            id: id.clone(),
            name: name.clone(),
            input: input_string(call.raw_input.as_ref()),
        });
        self.tools.insert(
            id.clone(),
            ToolState {
                name,
                title: call.title.clone(),
                kind,
                acp_kind: call.kind,
                is_mcp_meta,
                input: call.raw_input.clone(),
                started_at: None,
                ended: false,
            },
        );
        self.after_fields(
            &id,
            Some(&call.status),
            Some(&call.content),
            call.raw_output.as_ref(),
        )
    }

    fn on_tool_call_update(&mut self, update: &ToolCallUpdate) -> Vec<OutEvent> {
        let id = update.tool_call_id.0.to_string();
        if !self.tools.contains_key(&id) {
            let title = update.fields.title.clone().unwrap_or_else(|| id.clone());
            let call = ToolCall::new(update.tool_call_id.clone(), title)
                .kind(update.fields.kind.unwrap_or(ToolKind::Other))
                .raw_input(update.fields.raw_input.clone());
            let mut events = self.on_tool_call(&call);
            events.extend(self.on_tool_call_update(update));
            return events;
        }
        let f = &update.fields;
        self.apply_tool_fields(
            &id,
            f.kind,
            f.status,
            f.title.clone(),
            f.name.clone(),
            f.content.as_ref(),
            f.raw_input.clone(),
            f.raw_output.as_ref(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_tool_fields(
        &mut self,
        id: &str,
        acp_kind: Option<ToolKind>,
        status: Option<ToolCallStatus>,
        title: Option<String>,
        name: Option<String>,
        content: Option<&Vec<ToolCallContent>>,
        raw_input: Option<Value>,
        raw_output: Option<&Value>,
    ) -> Vec<OutEvent> {
        let servers = self.mcp_servers.clone();
        if let Some(state) = self.tools.get_mut(id) {
            if state.started_at.is_none() {
                if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
                    state.title = t;
                }
                if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
                    state.name = n;
                }
                if let Some(k) = acp_kind {
                    state.acp_kind = k;
                }
                state.kind = kind_for(
                    &servers,
                    state.acp_kind,
                    &state.name,
                    &state.title,
                    state.is_mcp_meta,
                );
            }
            if let Some(input) = raw_input.filter(|v| !is_empty_input(v)) {
                state.input = Some(input.clone());
                let input_text = input_string(Some(&input));
                patch_tool_use_input(&mut self.pending, &mut self.messages, id, &input_text);
            }
        }
        self.after_fields(id, status.as_ref(), content, raw_output)
    }

    fn after_fields(
        &mut self,
        id: &str,
        status: Option<&ToolCallStatus>,
        content: Option<&Vec<ToolCallContent>>,
        raw_output: Option<&Value>,
    ) -> Vec<OutEvent> {
        let mut events = Vec::new();
        let Some(state) = self.tools.get(id) else {
            return events;
        };
        if state.ended {
            return events;
        }
        let ready = matches!(
            status,
            Some(ToolCallStatus::InProgress | ToolCallStatus::Completed | ToolCallStatus::Failed)
        ) || state.input.as_ref().is_some_and(|v| !is_empty_input(v));
        if ready && state.started_at.is_none() {
            events.push(OutEvent::ToolStart {
                id: id.to_string(),
                name: state.name.clone(),
                kind: state.kind,
                input: input_string(state.input.as_ref()),
            });
            if let Some(s) = self.tools.get_mut(id) {
                s.started_at = Some(Instant::now());
            }
        }
        let content_text = content.map(|c| contents_text(c)).unwrap_or_default();
        match status {
            Some(ToolCallStatus::Completed) => {
                events.extend(self.end_tool(id, true, tool_output(&content_text, raw_output)));
            }
            Some(ToolCallStatus::Failed) => {
                events.extend(self.end_tool(id, false, tool_output(&content_text, raw_output)));
            }
            _ => {
                let kind = self.tools.get(id).map(|t| t.kind);
                if kind == Some("shell") && !content_text.is_empty() {
                    events.push(OutEvent::ShellChunk {
                        id: id.to_string(),
                        text: content_text,
                    });
                }
            }
        }
        events
    }

    fn end_tool(&mut self, id: &str, ok: bool, output: String) -> Vec<OutEvent> {
        let mut events = Vec::new();
        let Some(state) = self.tools.get_mut(id) else {
            return events;
        };
        if state.started_at.is_none() {
            events.push(OutEvent::ToolStart {
                id: id.to_string(),
                name: state.name.clone(),
                kind: state.kind,
                input: input_string(state.input.as_ref()),
            });
            state.started_at = Some(Instant::now());
        }
        state.ended = true;
        let duration_ms = state
            .started_at
            .map(|t| u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default();
        let name = state.name.clone();
        let kind = state.kind;
        events.push(OutEvent::ToolEnd {
            id: id.to_string(),
            name: name.clone(),
            kind,
            ok,
            duration_ms,
            output: output.clone(),
        });
        self.flush_assistant();
        self.messages
            .push(ConversationMessage::tool_result(id, name, output, !ok));
        events
    }

    fn push_text(&mut self, text: &str) {
        if let Some(ContentBlock::Text { text: last }) = self.pending.last_mut() {
            last.push_str(text);
        } else {
            self.pending.push(ContentBlock::Text {
                text: text.to_string(),
            });
        }
    }

    fn flush_assistant(&mut self) {
        if !self.pending.is_empty() {
            let blocks = std::mem::take(&mut self.pending);
            self.messages.push(ConversationMessage::assistant(blocks));
        }
    }

    fn contract_kind(
        &self,
        acp: ToolKind,
        name: &str,
        title: &str,
        mcp_meta: bool,
    ) -> &'static str {
        kind_for(&self.mcp_servers, acp, name, title, mcp_meta)
    }
}

/// Contract `kind` (existing `tool_process_kind` vocabulary). MCP wins over the ACP kind because
/// codex-acp reports MCP calls as `execute`.
fn kind_for(
    servers: &[String],
    acp: ToolKind,
    name: &str,
    title: &str,
    mcp_meta: bool,
) -> &'static str {
    if mcp_meta
        || servers
            .iter()
            .any(|s| is_mcp_name(s, name) || is_mcp_name(s, title))
    {
        return "mcp";
    }
    match acp {
        ToolKind::Execute => "shell",
        ToolKind::Edit | ToolKind::Delete | ToolKind::Move => "edit",
        ToolKind::Read => "read",
        ToolKind::Search => "search",
        _ => "tool",
    }
}

fn is_mcp_name(server: &str, s: &str) -> bool {
    s.starts_with(&format!("mcp.{server}."))
        || s.starts_with(&format!("mcp__{server}"))
        || s.starts_with(&format!("{server}_"))
}

fn meta_flag(meta: Option<&serde_json::Map<String, Value>>, key: &str) -> bool {
    meta.and_then(|m| m.get(key)).and_then(Value::as_bool) == Some(true)
}

fn is_empty_input(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

fn input_string(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "{}".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn patch_tool_use_input(
    pending: &mut [ContentBlock],
    messages: &mut [ConversationMessage],
    id: &str,
    input_text: &str,
) {
    let blocks = pending
        .iter_mut()
        .chain(messages.iter_mut().flat_map(|m| m.blocks.iter_mut()));
    for block in blocks {
        if let ContentBlock::ToolUse { id: bid, input, .. } = block {
            if bid == id {
                *input = input_text.to_string();
                return;
            }
        }
    }
}

fn block_text(block: &AcpBlock) -> String {
    match block {
        AcpBlock::Text(t) => t.text.clone(),
        AcpBlock::ResourceLink(l) => l.uri.clone(),
        _ => String::new(),
    }
}

fn contents_text(contents: &[ToolCallContent]) -> String {
    contents
        .iter()
        .filter_map(|c| match c {
            ToolCallContent::Content(c) => Some(block_text(&c.content)),
            ToolCallContent::Diff(d) => Some(format!("diff: {}", d.path.display())),
            _ => None,
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Content text first; else MCP-shaped `rawOutput` (`content[].text`, optionally under `result`);
/// else the raw JSON.
fn tool_output(content_text: &str, raw_output: Option<&Value>) -> String {
    if !content_text.is_empty() {
        return content_text.to_string();
    }
    let Some(raw) = raw_output else {
        return String::new();
    };
    if let Value::String(s) = raw {
        return s.clone();
    }
    let mcp = raw.get("result").unwrap_or(raw);
    if let Some(items) = mcp.get("content").and_then(Value::as_array) {
        let text: Vec<&str> = items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect();
        if !text.is_empty() {
            return text.join("\n");
        }
    }
    if let Some(s) = raw.get("output").and_then(Value::as_str) {
        return s.to_string();
    }
    raw.to_string()
}

/// ACP `PromptResponse.usage` → claw `TokenUsage`.
#[must_use]
pub fn token_usage(usage: &Usage) -> TokenUsage {
    let clamp = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
    TokenUsage {
        input_tokens: clamp(usage.input_tokens),
        output_tokens: clamp(usage.output_tokens),
        cache_creation_input_tokens: clamp(usage.cached_write_tokens.unwrap_or(0)),
        cache_read_input_tokens: clamp(usage.cached_read_tokens.unwrap_or(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn upd(v: Value) -> SessionUpdate {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn shell_chunks_failed_end_and_thinking_gate() {
        let mut m = TurnMapper::new(Vec::new(), false);
        assert!(m
            .on_update(&upd(json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"hmm"}})))
            .is_empty());
        let ev = m.on_update(&upd(json!({"sessionUpdate":"tool_call","toolCallId":"t1","title":"bash","kind":"execute","status":"in_progress","rawInput":{"command":"ls"}})));
        assert!(matches!(
            &ev[..],
            [OutEvent::ToolStart { kind: "shell", .. }]
        ));
        let ev = m.on_update(&upd(json!({"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"in_progress","content":[{"type":"content","content":{"type":"text","text":"a.txt"}}]})));
        assert_eq!(
            ev,
            vec![OutEvent::ShellChunk {
                id: "t1".into(),
                text: "a.txt".into()
            }]
        );
        let ev = m.on_update(&upd(json!({"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"failed","rawOutput":"exit 1"})));
        assert!(
            matches!(&ev[..], [OutEvent::ToolEnd { ok: false, output, .. }] if output == "exit 1")
        );
        let (t, tail) = m.finish(None);
        assert!(tail.is_empty());
        assert!(matches!(
            &t.messages[1].blocks[..],
            [ContentBlock::ToolResult { is_error: true, .. }]
        ));
    }

    #[test]
    fn unfinished_tool_is_closed_as_error_on_finish() {
        let mut m = TurnMapper::new(Vec::new(), true);
        let ev = m.on_update(&upd(
            json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"hmm"}}),
        ));
        assert_eq!(ev, vec![OutEvent::ThinkingDelta("hmm".into())]);
        m.on_update(&upd(json!({"sessionUpdate":"tool_call","toolCallId":"t9","title":"read","kind":"read","status":"pending"})));
        let (t, tail) = m.finish(None);
        assert!(matches!(
            &tail[..],
            [
                OutEvent::ToolStart { kind: "read", .. },
                OutEvent::ToolEnd { ok: false, .. }
            ]
        ));
        assert_eq!(t.messages.len(), 2);
        assert_eq!(t.tool_calls, 1);
    }
}
