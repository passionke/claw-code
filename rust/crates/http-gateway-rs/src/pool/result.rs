//! Parse `claw gateway-solve-once` stdout (JSON contract). Author: kejiqing

use serde_json::{json, Value};

/// Parsed fields used to build [`crate::SolveResponse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGatewaySolvePayload {
    pub claw_exit_code: i32,
    pub output_text: String,
    pub output_json: Option<Value>,
}

/// Parse stdout from `docker exec … claw gateway-solve-once` (JSON line or fallback).
pub fn parse_gateway_solve_exec_stdout(
    stdout: &str,
    fallback_exit_code: i32,
) -> ParsedGatewaySolvePayload {
    let trimmed = stdout.trim();
    let parsed: Value = serde_json::from_str(trimmed).unwrap_or_else(|_| {
        json!({
            "clawExitCode": fallback_exit_code,
            "outputText": stdout,
            "outputJson": Value::Null,
        })
    });
    let raw_exit = parsed
        .get("clawExitCode")
        .and_then(Value::as_i64)
        .unwrap_or(i64::from(fallback_exit_code));
    let claw_exit_code = i32::try_from(raw_exit).unwrap_or(fallback_exit_code);
    let mut output_text = parsed
        .get("outputText")
        .and_then(|v| v.as_str())
        .or_else(|| parsed.get("error").and_then(|v| v.as_str()))
        .unwrap_or("")
        .to_string();
    let output_json = parsed
        .get("outputJson")
        .cloned()
        .or_else(|| parsed.get("error").map(|_| parsed.clone()));
    output_text = normalize_user_visible_output_text(&output_text, &output_json);
    ParsedGatewaySolvePayload {
        claw_exit_code,
        output_text,
        output_json,
    }
}

/// Legacy workers put the full solve JSON bundle in `outputText`; prefer `message` / `outputJson.message`.
pub fn normalize_user_visible_output_text(
    output_text: &str,
    output_json: &Option<Value>,
) -> String {
    if let Some(msg) = output_json
        .as_ref()
        .and_then(|j| j.get("message"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return msg.to_string();
    }
    if let Ok(v) = serde_json::from_str::<Value>(output_text.trim()) {
        if let Some(msg) = v.get("message").and_then(Value::as_str) {
            if v.get("iterations").is_some() || v.get("usage").is_some() {
                return msg.to_string();
            }
        }
    }
    output_text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compact_json_line() {
        let raw = r#"{"clawExitCode":0,"outputText":"hi","outputJson":{"x":1}}"#;
        let p = parse_gateway_solve_exec_stdout(raw, -1);
        assert_eq!(p.claw_exit_code, 0);
        assert_eq!(p.output_text, "hi");
        assert_eq!(p.output_json, Some(json!({"x": 1})));
    }

    #[test]
    fn parses_explicit_null_output_json() {
        let p = parse_gateway_solve_exec_stdout(
            r#"{"clawExitCode":0,"outputText":"x","outputJson":null}"#,
            -1,
        );
        assert_eq!(p.output_json, Some(Value::Null));
    }

    #[test]
    fn falls_back_when_stdout_not_json() {
        let p = parse_gateway_solve_exec_stdout("not json\n", 42);
        assert_eq!(p.claw_exit_code, 42);
        // Fallback uses raw stdout (same as legacy gateway solve path).
        assert_eq!(p.output_text, "not json\n");
        assert_eq!(p.output_json, Some(Value::Null));
    }

    #[test]
    fn uses_exit_code_when_json_omits_claw_exit() {
        let p = parse_gateway_solve_exec_stdout(r#"{"outputText":"x"}"#, 7);
        assert_eq!(p.claw_exit_code, 7);
        assert_eq!(p.output_text, "x");
    }

    #[test]
    fn unwraps_legacy_json_bundle_in_output_text() {
        let bundle = json!({
            "model": "m",
            "iterations": 1,
            "message": "Hello!",
            "usage": {}
        });
        let line = json!({
            "clawExitCode": 0,
            "outputText": bundle.to_string(),
            "outputJson": bundle,
        });
        let p = parse_gateway_solve_exec_stdout(&line.to_string(), 0);
        assert_eq!(p.output_text, "Hello!");
    }

    #[test]
    fn keeps_gateway_error_payload() {
        let p = parse_gateway_solve_exec_stdout(
            r#"{"clawExitCode":1,"error":"runtime prompt failed","httpStatusHint":500}"#,
            1,
        );
        assert_eq!(p.claw_exit_code, 1);
        assert_eq!(p.output_text, "runtime prompt failed");
        assert_eq!(
            p.output_json,
            Some(json!({
                "clawExitCode": 1,
                "error": "runtime prompt failed",
                "httpStatusHint": 500
            }))
        );
    }
}
