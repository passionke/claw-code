//! Parse `claw gateway-solve-once` stdout (NDJSON events + terminal `solve.done`). Author: kejiqing

use gateway_solve_turn::parse_stdout_line;
use serde_json::Value;

/// Parsed fields used to build [`crate::SolveResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGatewaySolvePayload {
    pub claw_exit_code: i32,
    pub output_text: String,
    pub output_json: Option<Value>,
}

fn payload_from_solve_json(parsed: &Value, fallback_exit_code: i32) -> ParsedGatewaySolvePayload {
    let raw_exit = parsed
        .get("clawExitCode")
        .and_then(Value::as_i64)
        .unwrap_or(i64::from(fallback_exit_code));
    let claw_exit_code = i32::try_from(raw_exit).unwrap_or(fallback_exit_code);
    let output_text = parsed
        .get("outputText")
        .and_then(|v| v.as_str())
        .or_else(|| parsed.get("error").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let output_json = parsed
        .get("outputJson")
        .cloned()
        .or_else(|| parsed.get("error").map(|_| parsed.clone()));
    ParsedGatewaySolvePayload {
        claw_exit_code,
        output_text,
        output_json,
    }
}

/// Parse stdout from `docker exec … claw gateway-solve-once`.
///
/// Structured lines use [`gateway_solve_turn::GATEWAY_STDOUT_LINE_PREFIX`]; the last `solve.done`
/// event (or legacy single JSON object with `clawExitCode`) wins.
pub fn parse_gateway_solve_exec_stdout(
    stdout: &str,
    fallback_exit_code: i32,
) -> ParsedGatewaySolvePayload {
    let mut last_solve_done: Option<Value> = None;
    let mut last_legacy: Option<Value> = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(v) = parse_stdout_line(trimmed) {
            if v.get("ev").and_then(Value::as_str) == Some("solve.done") {
                last_solve_done = Some(v);
            }
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            if v.get("clawExitCode").is_some() {
                last_legacy = Some(v);
            }
        }
    }
    if let Some(parsed) = last_solve_done.or(last_legacy) {
        return payload_from_solve_json(&parsed, fallback_exit_code);
    }
    let trimmed = stdout.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        if v.get("clawExitCode").is_some() {
            return payload_from_solve_json(&v, fallback_exit_code);
        }
    }
    ParsedGatewaySolvePayload {
        claw_exit_code: fallback_exit_code,
        output_text: stdout.to_string(),
        output_json: Some(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_solve_turn::GATEWAY_STDOUT_LINE_PREFIX;
    use serde_json::json;

    #[test]
    fn parses_solve_done_after_deltas() {
        let raw = format!(
            "{GATEWAY_STDOUT_LINE_PREFIX}{}\n{GATEWAY_STDOUT_LINE_PREFIX}{}\n",
            json!({"ev":"report.delta","text":"# 报告"}),
            json!({"ev":"solve.done","clawExitCode":0,"outputText":"ok","outputJson":{"message":"ok"}})
        );
        let p = parse_gateway_solve_exec_stdout(&raw, -1);
        assert_eq!(p.claw_exit_code, 0);
        assert_eq!(p.output_text, "ok");
    }

    #[test]
    fn parses_compact_json_line_legacy() {
        let raw = r#"{"clawExitCode":0,"outputText":"hi","outputJson":{"x":1}}"#;
        let p = parse_gateway_solve_exec_stdout(raw, -1);
        assert_eq!(p.claw_exit_code, 0);
        assert_eq!(p.output_text, "hi");
        assert_eq!(p.output_json, Some(json!({"x": 1})));
    }

    #[test]
    fn falls_back_when_stdout_not_json() {
        let p = parse_gateway_solve_exec_stdout("not json\n", 42);
        assert_eq!(p.claw_exit_code, 42);
        assert_eq!(p.output_text, "not json\n");
    }
}
