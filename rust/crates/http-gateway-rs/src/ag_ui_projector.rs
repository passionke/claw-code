//! AG-UI event projection from LiveReportHub (+ A2UI claw-process/v1). Author: kejiqing

use serde_json::{json, Value};

use crate::pool::{AskUserPending, HubMsg, ProcessEvent};

/// One process step tracked for `claw-process/v1`. Author: kejiqing
#[derive(Debug, Clone)]
pub struct ProcessStep {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub duration_ms: Option<u64>,
    pub summary: String,
}

/// Build / update process steps from a Hub process event. Author: kejiqing
pub fn apply_process_event(steps: &mut Vec<ProcessStep>, pe: &ProcessEvent) {
    let p = &pe.payload;
    let tool_call_id = p
        .get("toolCallId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match pe.ev.as_str() {
        "tool.start" => {
            let kind = p
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let title = p
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let summary = p
                .get("argsSummary")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(existing) = steps.iter_mut().find(|s| s.id == tool_call_id) {
                existing.kind = kind;
                existing.title = title;
                existing.status = "running".into();
                existing.summary = summary;
            } else {
                steps.push(ProcessStep {
                    id: if tool_call_id.is_empty() {
                        format!("step_{}", steps.len() + 1)
                    } else {
                        tool_call_id
                    },
                    kind,
                    title,
                    status: "running".into(),
                    duration_ms: None,
                    summary,
                });
            }
        }
        "tool.end" => {
            let status = p
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("ok")
                .to_string();
            let duration_ms = p.get("durationMs").and_then(Value::as_u64);
            let summary = p
                .get("resultSummary")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(existing) = steps.iter_mut().find(|s| s.id == tool_call_id) {
                existing.status = status;
                existing.duration_ms = duration_ms;
                if !summary.is_empty() {
                    existing.summary = summary;
                }
            } else {
                let name = p
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let kind = p
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                steps.push(ProcessStep {
                    id: if tool_call_id.is_empty() {
                        format!("step_{}", steps.len() + 1)
                    } else {
                        tool_call_id
                    },
                    kind,
                    title: name,
                    status,
                    duration_ms,
                    summary,
                });
            }
        }
        "progress" => {
            let message = p
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if message.is_empty() {
                return;
            }
            steps.push(ProcessStep {
                id: format!("progress_{}", steps.len() + 1),
                kind: p
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("progress")
                    .to_string(),
                title: message.clone(),
                status: "ok".into(),
                duration_ms: None,
                summary: message,
            });
        }
        _ => {}
    }
}

/// `claw-process/v1` A2UI surface. Author: kejiqing
#[must_use]
pub fn build_process_a2ui(turn_id: &str, steps: &[ProcessStep]) -> Value {
    let components: Vec<Value> = steps
        .iter()
        .map(|s| {
            json!({
                "id": format!("step-{}", s.id),
                "component": "ProcessStep",
                "kind": s.kind,
                "title": s.title,
                "status": s.status,
                "durationMs": s.duration_ms,
                "summary": s.summary,
            })
        })
        .collect();
    json!({
        "version": "0.8",
        "catalogId": "claw-process/v1",
        "surfaceId": format!("process-{turn_id}"),
        "components": components,
    })
}

#[must_use]
pub fn ag_ui_run_started(thread_id: &str, run_id: &str) -> Value {
    json!({
        "type": "RUN_STARTED",
        "threadId": thread_id,
        "runId": run_id,
    })
}

#[must_use]
pub fn ag_ui_run_finished(thread_id: &str, run_id: &str) -> Value {
    json!({
        "type": "RUN_FINISHED",
        "threadId": thread_id,
        "runId": run_id,
    })
}

#[must_use]
pub fn ag_ui_custom_a2ui(a2ui: &Value) -> Value {
    json!({
        "type": "CUSTOM",
        "name": "a2ui",
        "value": a2ui,
    })
}

#[must_use]
pub fn ag_ui_from_tool_start(payload: &Value) -> Value {
    json!({
        "type": "TOOL_CALL_START",
        "toolCallId": payload.get("toolCallId").and_then(Value::as_str).unwrap_or(""),
        "toolCallName": payload.get("name").and_then(Value::as_str).unwrap_or(""),
        "title": payload.get("title").and_then(Value::as_str).unwrap_or(""),
        "kind": payload.get("kind").and_then(Value::as_str).unwrap_or("tool"),
    })
}

#[must_use]
pub fn ag_ui_from_tool_end(payload: &Value) -> Value {
    json!({
        "type": "TOOL_CALL_END",
        "toolCallId": payload.get("toolCallId").and_then(Value::as_str).unwrap_or(""),
        "toolCallName": payload.get("name").and_then(Value::as_str).unwrap_or(""),
        "status": payload.get("status").and_then(Value::as_str).unwrap_or("ok"),
        "durationMs": payload.get("durationMs"),
    })
}

#[must_use]
pub fn ag_ui_tool_result(payload: &Value) -> Value {
    json!({
        "type": "TOOL_CALL_RESULT",
        "toolCallId": payload.get("toolCallId").and_then(Value::as_str).unwrap_or(""),
        "content": payload.get("resultSummary").and_then(Value::as_str).unwrap_or(""),
    })
}

#[must_use]
pub fn ag_ui_state_delta_progress(payload: &Value) -> Value {
    json!({
        "type": "STATE_DELTA",
        "delta": [{
            "op": "add",
            "path": "/progress/-",
            "value": {
                "kind": payload.get("kind").and_then(Value::as_str).unwrap_or("progress"),
                "message": payload.get("message").and_then(Value::as_str).unwrap_or(""),
                "tsMs": payload.get("tsMs"),
            }
        }]
    })
}

#[must_use]
pub fn ag_ui_from_ask_user(pending: &AskUserPending) -> Value {
    let a2ui = if pending.a2ui.is_null() {
        json!({
            "version": "0.8",
            "catalogId": "claw-ask/v1",
            "surfaceId": format!("ask-{}", pending.question_id),
            "components": []
        })
    } else {
        pending.a2ui.clone()
    };
    ag_ui_custom_a2ui(&a2ui)
}

/// Map one HubMsg to zero or more AG-UI JSON events (excluding RUN_*). Author: kejiqing
pub fn project_hub_msg(turn_id: &str, msg: &HubMsg, steps: &mut Vec<ProcessStep>) -> Vec<Value> {
    match msg {
        HubMsg::Process(pe) => {
            apply_process_event(steps, pe);
            let mut out = Vec::new();
            match pe.ev.as_str() {
                "tool.start" => out.push(ag_ui_from_tool_start(&pe.payload)),
                "tool.end" => {
                    out.push(ag_ui_from_tool_end(&pe.payload));
                    out.push(ag_ui_tool_result(&pe.payload));
                }
                "progress" => out.push(ag_ui_state_delta_progress(&pe.payload)),
                _ => {}
            }
            out.push(ag_ui_custom_a2ui(&build_process_a2ui(turn_id, steps)));
            out
        }
        HubMsg::AskUser(pending) => vec![ag_ui_from_ask_user(pending)],
        HubMsg::AskUserCleared => vec![json!({
            "type": "CUSTOM",
            "name": "a2ui.cleared",
            "value": { "catalogId": "claw-ask/v1" }
        })],
        HubMsg::Delta(_) | HubMsg::SolveDone => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn process_a2ui_from_tool_lifecycle() {
        let mut steps = Vec::new();
        apply_process_event(
            &mut steps,
            &ProcessEvent {
                ev: "tool.start".into(),
                payload: json!({
                    "toolCallId": "tc1",
                    "name": "Bash",
                    "kind": "shell",
                    "title": "Bash: ls",
                    "argsSummary": "ls"
                }),
            },
        );
        apply_process_event(
            &mut steps,
            &ProcessEvent {
                ev: "tool.end".into(),
                payload: json!({
                    "toolCallId": "tc1",
                    "name": "Bash",
                    "status": "ok",
                    "durationMs": 12,
                    "resultSummary": "ok"
                }),
            },
        );
        let a2ui = build_process_a2ui("T1", &steps);
        assert_eq!(a2ui["catalogId"], "claw-process/v1");
        assert_eq!(a2ui["components"][0]["status"], "ok");
    }
}
