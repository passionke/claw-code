//! Line-delimited stdout events from `claw gateway-solve-once` for pool exec streaming. Author: kejiqing

use std::io::{self, Write};

use serde::Serialize;
use serde_json::Value;

/// Reset stdout delegate flags at turn start. Author: kejiqing
pub fn reset_delegate_stdout_state() {}

/// Prefix for every structured stdout line (plain logs must not use this prefix).
pub const GATEWAY_STDOUT_LINE_PREFIX: &str = "__CLAW_GATEWAY_STDOUT__";

#[derive(Debug, Serialize)]
struct StdoutEnvelope<'a> {
    ev: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
    #[serde(rename = "emitSeq", skip_serializing_if = "Option::is_none")]
    emit_seq: Option<u64>,
    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none")]
    session_id: Option<&'a str>,
    #[serde(rename = "turnId", skip_serializing_if = "Option::is_none")]
    turn_id: Option<&'a str>,
    #[serde(rename = "projId", skip_serializing_if = "Option::is_none")]
    proj_id: Option<i64>,
    #[serde(rename = "delegateProjId", skip_serializing_if = "Option::is_none")]
    delegate_proj_id: Option<i64>,
    #[serde(rename = "clawExitCode", skip_serializing_if = "Option::is_none")]
    claw_exit_code: Option<i32>,
    #[serde(rename = "outputText", skip_serializing_if = "Option::is_none")]
    output_text: Option<&'a str>,
    #[serde(rename = "outputJson", skip_serializing_if = "Option::is_none")]
    output_json: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
    #[serde(rename = "httpStatusHint", skip_serializing_if = "Option::is_none")]
    http_status_hint: Option<u16>,
}

fn emit_line(value: &StdoutEnvelope<'_>) -> io::Result<()> {
    let body = serde_json::to_string(value).map_err(|e| io::Error::other(e.to_string()))?;
    writeln!(io::stdout(), "{GATEWAY_STDOUT_LINE_PREFIX}{body}")?;
    io::stdout().flush()
}

/// `{"ev":"report.delta","text":"…"}` — pool exec reads stdout line-by-line and forwards to gateway hub.
pub fn emit_report_delta(text: &str) -> io::Result<()> {
    if text.is_empty() {
        return Ok(());
    }
    let emit_seq = api::sse_burst_trace::log_worker_emit(text);
    emit_line(&StdoutEnvelope {
        ev: "report.delta",
        text: Some(text),
        emit_seq: Some(emit_seq),
        session_id: None,
        turn_id: None,
        proj_id: None,
        delegate_proj_id: None,
        claw_exit_code: None,
        output_text: None,
        output_json: None,
        error: None,
        http_status_hint: None,
    })
}

/// Register active specialist delegate for gateway live SSE fan-in. Author: kejiqing
pub fn emit_delegate_active(
    session_id: &str,
    turn_id: &str,
    proj_id: i64,
    delegate_proj_id: i64,
) -> io::Result<()> {
    emit_line(&StdoutEnvelope {
        ev: "delegate.active",
        text: None,
        emit_seq: None,
        session_id: Some(session_id),
        turn_id: Some(turn_id),
        proj_id: Some(proj_id),
        delegate_proj_id: Some(delegate_proj_id),
        claw_exit_code: None,
        output_text: None,
        output_json: None,
        error: None,
        http_status_hint: None,
    })
}

/// Clear active delegate; gateway archives progress before drop. Author: kejiqing
pub fn emit_delegate_clear() -> io::Result<()> {
    emit_line(&StdoutEnvelope {
        ev: "delegate.clear",
        text: None,
        emit_seq: None,
        session_id: None,
        turn_id: None,
        proj_id: None,
        delegate_proj_id: None,
        claw_exit_code: None,
        output_text: None,
        output_json: None,
        error: None,
        http_status_hint: None,
    })
}

/// Terminal solve result (last structured stdout line on stdout).
pub fn emit_solve_done(
    claw_exit_code: i32,
    output_text: &str,
    output_json: Option<&Value>,
) -> io::Result<()> {
    emit_line(&StdoutEnvelope {
        ev: "solve.done",
        text: None,
        emit_seq: None,
        session_id: None,
        turn_id: None,
        proj_id: None,
        delegate_proj_id: None,
        claw_exit_code: Some(claw_exit_code),
        output_text: Some(output_text),
        output_json,
        error: None,
        http_status_hint: None,
    })
}

pub fn emit_solve_error(message: &str, http_status_hint: u16) -> io::Result<()> {
    emit_line(&StdoutEnvelope {
        ev: "solve.done",
        text: None,
        emit_seq: None,
        session_id: None,
        turn_id: None,
        proj_id: None,
        delegate_proj_id: None,
        claw_exit_code: Some(1),
        output_text: None,
        output_json: None,
        error: Some(message),
        http_status_hint: Some(http_status_hint),
    })
}

/// Emit an arbitrary structured stdout event (must include `"ev"`). Author: kejiqing
pub fn emit_raw_json(value: &Value) -> io::Result<()> {
    let body = serde_json::to_string(value).map_err(|e| io::Error::other(e.to_string()))?;
    writeln!(io::stdout(), "{GATEWAY_STDOUT_LINE_PREFIX}{body}")?;
    io::stdout().flush()
}

const TOOL_SUMMARY_MAX: usize = 240;

fn truncate_summary(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= TOOL_SUMMARY_MAX {
        return t.to_string();
    }
    format!(
        "{}…",
        t.chars().take(TOOL_SUMMARY_MAX.saturating_sub(1)).collect::<String>()
    )
}

/// Map tool name → process disclosure kind. Author: kejiqing
#[must_use]
pub fn tool_process_kind(tool_name: &str) -> &'static str {
    let n = tool_name.to_ascii_lowercase();
    if n.contains("grep") || n.contains("search") || n.contains("glob") {
        "search"
    } else if n.contains("read") {
        "read"
    } else if n.contains("write") || n.contains("edit") {
        "edit"
    } else if n == "bash" || n.contains("shell") {
        "shell"
    } else if n.contains("mcp") || n == "mcp" {
        "mcp"
    } else if n.contains("delegate") {
        "delegate"
    } else {
        "tool"
    }
}

fn tool_title(tool_name: &str, args_summary: &str) -> String {
    let kind = tool_process_kind(tool_name);
    match kind {
        "search" => {
            if args_summary.is_empty() {
                format!("Searching ({tool_name})")
            } else {
                format!("Searching {args_summary}")
            }
        }
        "read" => {
            if args_summary.is_empty() {
                format!("Reading ({tool_name})")
            } else {
                format!("Reading {args_summary}")
            }
        }
        "edit" => {
            if args_summary.is_empty() {
                format!("Editing ({tool_name})")
            } else {
                format!("Editing {args_summary}")
            }
        }
        "shell" => {
            if args_summary.is_empty() {
                "Bash".into()
            } else {
                format!("Bash: {args_summary}")
            }
        }
        "mcp" => {
            if args_summary.is_empty() {
                format!("MCP {tool_name}")
            } else {
                format!("MCP {tool_name}: {args_summary}")
            }
        }
        "delegate" => format!("Delegate → {args_summary}"),
        _ => {
            if args_summary.is_empty() {
                tool_name.to_string()
            } else {
                format!("{tool_name}: {args_summary}")
            }
        }
    }
}

fn args_summary_from_input(tool_name: &str, input: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(input) else {
        return truncate_summary(input);
    };
    let n = tool_name.to_ascii_lowercase();
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| v.get(*k).and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if n == "bash" || n.contains("shell") {
        return truncate_summary(&pick(&["command", "cmd"]).unwrap_or_default());
    }
    if n.contains("grep") || n.contains("search") {
        return truncate_summary(
            &pick(&["pattern", "query", "q", "keyword"]).unwrap_or_default(),
        );
    }
    if n.contains("read") || n.contains("write") || n.contains("edit") || n.contains("glob") {
        return truncate_summary(&pick(&["path", "file_path", "file", "target"]).unwrap_or_default());
    }
    if let Some(obj) = v.as_object() {
        if let Some(s) = obj
            .values()
            .find_map(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return truncate_summary(s);
        }
    }
    truncate_summary(&v.to_string())
}

/// Emit `tool.start` for AG-UI / process disclosure. Author: kejiqing
pub fn emit_tool_start(tool_call_id: &str, tool_name: &str, input: &str) -> io::Result<()> {
    let args_summary = args_summary_from_input(tool_name, input);
    let title = tool_title(tool_name, &args_summary);
    emit_raw_json(&serde_json::json!({
        "ev": "tool.start",
        "toolCallId": tool_call_id,
        "name": tool_name,
        "kind": tool_process_kind(tool_name),
        "title": title,
        "argsSummary": args_summary,
    }))
}

/// Emit `tool.end` for AG-UI / process disclosure. Author: kejiqing
pub fn emit_tool_end(
    tool_call_id: &str,
    tool_name: &str,
    ok: bool,
    duration_ms: u64,
    result: &str,
) -> io::Result<()> {
    emit_raw_json(&serde_json::json!({
        "ev": "tool.end",
        "toolCallId": tool_call_id,
        "name": tool_name,
        "kind": tool_process_kind(tool_name),
        "status": if ok { "ok" } else { "error" },
        "durationMs": duration_ms,
        "resultSummary": truncate_summary(result),
    }))
}

/// Parse one stdout line; returns `Some(event)` when prefixed.
#[must_use]
pub fn parse_stdout_line(line: &str) -> Option<Value> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix(GATEWAY_STDOUT_LINE_PREFIX)?;
    serde_json::from_str(rest).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_prefixed_line() {
        let line = format!(
            "{GATEWAY_STDOUT_LINE_PREFIX}{}",
            json!({"ev":"report.delta","text":"hi"})
        );
        let v = parse_stdout_line(&line).expect("parse");
        assert_eq!(v.get("ev").and_then(|x| x.as_str()), Some("report.delta"));
    }

    #[test]
    fn parse_delegate_active_line() {
        let line = format!(
            "{GATEWAY_STDOUT_LINE_PREFIX}{}",
            json!({
                "ev":"delegate.active",
                "sessionId":"dgt_x",
                "turnId":"T_y",
                "projId":99012,
                "delegateProjId":99012
            })
        );
        let v = parse_stdout_line(&line).expect("parse");
        assert_eq!(
            v.get("ev").and_then(|x| x.as_str()),
            Some("delegate.active")
        );
        assert_eq!(v.get("sessionId").and_then(|x| x.as_str()), Some("dgt_x"));
    }
}
