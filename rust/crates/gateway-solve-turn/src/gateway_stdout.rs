//! Line-delimited stdout events from `claw gateway-solve-once` for pool exec streaming. Author: kejiqing

use std::io::{self, Write};

use serde::Serialize;
use serde_json::Value;

/// Prefix for every structured stdout line (plain logs must not use this prefix).
pub const GATEWAY_STDOUT_LINE_PREFIX: &str = "__CLAW_GATEWAY_STDOUT__";

#[derive(Debug, Serialize)]
struct StdoutEnvelope<'a> {
    ev: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<&'a str>,
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
    api::sse_burst_trace::log_worker_emit(text.len());
    emit_line(&StdoutEnvelope {
        ev: "report.delta",
        text: Some(text),
        claw_exit_code: None,
        output_text: None,
        output_json: None,
        error: None,
        http_status_hint: None,
    })
}

/// Terminal solve result (last structured line on stdout).
pub fn emit_solve_done(
    claw_exit_code: i32,
    output_text: &str,
    output_json: Option<&Value>,
) -> io::Result<()> {
    emit_line(&StdoutEnvelope {
        ev: "solve.done",
        text: None,
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
        claw_exit_code: Some(1),
        output_text: None,
        output_json: None,
        error: Some(message),
        http_status_hint: Some(http_status_hint),
    })
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
}
