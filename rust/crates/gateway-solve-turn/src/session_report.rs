//! Read final assistant report text from gateway session jsonl (no LLM polish). Author: kejiqing

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::assistant_stream_spill::strip_report_start_marker;
use crate::gateway_solve_session_persistence_path;

/// Concatenate all assistant `text` blocks in jsonl order (same basis as solve `outputJson.message`).
pub fn final_assistant_report_text_from_jsonl(session_home: &Path) -> Result<String, String> {
    let path = gateway_solve_session_persistence_path(session_home);
    if !path.is_file() {
        return Ok(String::new());
    }
    let contents =
        fs::read_to_string(&path).map_err(|e| format!("read session jsonl failed: {e}"))?;
    let mut parts = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(msg) = record.get("message") else {
            continue;
        };
        if msg.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(blocks) = msg.get("blocks").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    if !text.is_empty() {
                        parts.push(text.to_string());
                    }
                }
            }
        }
    }
    Ok(strip_report_start_marker(&parts.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn extracts_assistant_text_blocks() {
        let dir = std::env::temp_dir().join(format!(
            "claw-jsonl-report-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let claw = dir.join(".claw");
        std::fs::create_dir_all(&claw).unwrap();
        let jsonl = claw.join("gateway-solve-session.jsonl");
        std::fs::write(
            &jsonl,
            r#"{"type":"message","message":{"role":"user","blocks":[{"type":"text","text":"hi"}]}}
{"type":"message","message":{"role":"assistant","blocks":[{"type":"text","text":"part1"},{"type":"text","text":"part2"}]}}
"#,
        )
        .unwrap();
        let text = final_assistant_report_text_from_jsonl(Path::new(&dir)).unwrap();
        assert_eq!(text, "part1\npart2");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
