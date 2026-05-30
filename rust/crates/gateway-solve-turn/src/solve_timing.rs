//! Append-only solve timing events (`.claw/solve-timing-events.ndjson`). Author: kejiqing

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use runtime::TurnTimingSink;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const SOLVE_TIMING_EVENTS_REL: &str = ".claw/solve-timing-events.ndjson";
const MAX_EVENT_LINES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SolveTimingEvent {
    pub kind: String,
    #[serde(rename = "tsMs")]
    pub ts_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "turnId")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "toolUseId")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "toolName")]
    pub tool_name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "durationMs"
    )]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "isError")]
    pub is_error: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "outputSize"
    )]
    pub output_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "inputSize")]
    pub input_size: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "pendingToolUseCount"
    )]
    pub pending_tool_use_count: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "inputTokens"
    )]
    pub input_tokens: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "outputTokens"
    )]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct SolveTimingRecorder {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl SolveTimingRecorder {
    pub fn new(session_home: &Path) -> Self {
        Self {
            path: session_home.join(SOLVE_TIMING_EVENTS_REL),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn append(&self, mut event: SolveTimingEvent) -> Result<(), String> {
        if event.ts_ms == 0 {
            event.ts_ms = now_ms();
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| String::from("solve timing lock poisoned"))?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create timing dir: {e}"))?;
        }
        let line = serde_json::to_string(&event).map_err(|e| format!("serialize timing: {e}"))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("open timing file: {e}"))?;
        writeln!(file, "{line}").map_err(|e| format!("append timing: {e}"))?;
        trim_ndjson_file(&self.path)?;
        Ok(())
    }

    pub fn record_direct_tool(
        &self,
        tool_name: &str,
        duration_ms: u128,
        is_error: bool,
    ) -> Result<(), String> {
        self.append(SolveTimingEvent {
            kind: "tool_execution_finished".to_string(),
            ts_ms: now_ms(),
            iteration: None,
            turn_id: None,
            tool_use_id: None,
            tool_name: Some(tool_name.to_string()),
            duration_ms: Some(i64::try_from(duration_ms).unwrap_or(i64::MAX)),
            is_error: Some(is_error),
            output_size: None,
            input_size: None,
            pending_tool_use_count: None,
            input_tokens: None,
            output_tokens: None,
            source: Some("direct".to_string()),
            error: None,
        })
    }
}

impl TurnTimingSink for SolveTimingRecorder {
    fn emit(&self, kind: &str, attributes: Map<String, Value>) {
        let event = SolveTimingEvent {
            kind: kind.to_string(),
            ts_ms: now_ms(),
            iteration: u64_attr(&attributes, "iteration"),
            turn_id: str_attr(&attributes, "turn_id"),
            tool_use_id: str_attr(&attributes, "tool_use_id"),
            tool_name: str_attr(&attributes, "tool_name"),
            duration_ms: i64_attr(&attributes, "duration_ms"),
            is_error: bool_attr(&attributes, "is_error"),
            output_size: u64_attr(&attributes, "output_size"),
            input_size: u64_attr(&attributes, "input_size"),
            pending_tool_use_count: u64_attr(&attributes, "pending_tool_use_count"),
            input_tokens: u64_attr(&attributes, "input_tokens"),
            output_tokens: u64_attr(&attributes, "output_tokens"),
            source: str_attr(&attributes, "source"),
            error: str_attr(&attributes, "error"),
        };
        let _ = self.append(event);
    }
}

#[must_use]
pub fn solve_timing_events_path(session_home: &Path) -> PathBuf {
    session_home.join(SOLVE_TIMING_EVENTS_REL)
}

pub fn truncate_solve_timing_events(session_home: &Path) -> Result<(), String> {
    let path = solve_timing_events_path(session_home);
    if path.is_file() {
        fs::remove_file(path).map_err(|e| format!("remove solve timing events failed: {e}"))?;
    }
    Ok(())
}

/// Events whose end timestamp falls in `[from_ms, to_ms]` (inclusive).
#[must_use]
pub fn filter_solve_timing_events_for_window(
    events: &[SolveTimingEvent],
    from_ms: i64,
    to_ms: i64,
) -> Vec<SolveTimingEvent> {
    events
        .iter()
        .filter(|ev| {
            let end = ev.ts_ms;
            let start = ev.duration_ms.map(|d| end.saturating_sub(d)).unwrap_or(end);
            end >= from_ms && start <= to_ms
        })
        .cloned()
        .collect()
}

/// Append a single bootstrap milestone (gateway pool / worker cold start). Author: kejiqing
pub fn append_solve_timing_point(
    session_home: &Path,
    kind: &str,
    turn_id: Option<&str>,
) -> Result<(), String> {
    SolveTimingRecorder::new(session_home).append(SolveTimingEvent {
        kind: kind.to_string(),
        ts_ms: now_ms(),
        iteration: None,
        turn_id: turn_id.map(str::to_string),
        tool_use_id: None,
        tool_name: None,
        duration_ms: None,
        is_error: None,
        output_size: None,
        input_size: None,
        pending_tool_use_count: None,
        input_tokens: None,
        output_tokens: None,
        source: Some("bootstrap".to_string()),
        error: None,
    })
}

pub fn read_solve_timing_events(
    session_home: &Path,
    limit: usize,
) -> Result<Vec<SolveTimingEvent>, String> {
    let path = solve_timing_events_path(session_home);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path).map_err(|e| format!("read timing events: {e}"))?;
    let mut out = Vec::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<SolveTimingEvent>(line) {
            out.push(entry);
        }
    }
    if out.len() > limit {
        out = out.split_off(out.len() - limit);
    }
    Ok(out)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn trim_ndjson_file(path: &Path) -> Result<(), String> {
    let contents = fs::read_to_string(path).map_err(|e| format!("read timing tail failed: {e}"))?;
    let lines: Vec<&str> = contents.lines().collect();
    if lines.len() <= MAX_EVENT_LINES {
        return Ok(());
    }
    let tail = lines[lines.len() - MAX_EVENT_LINES..].join("\n");
    fs::write(path, format!("{tail}\n")).map_err(|e| format!("trim timing events failed: {e}"))?;
    Ok(())
}

fn str_attr(attrs: &Map<String, Value>, key: &str) -> Option<String> {
    attrs.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn u64_attr(attrs: &Map<String, Value>, key: &str) -> Option<u64> {
    attrs.get(key).and_then(Value::as_u64)
}

fn i64_attr(attrs: &Map<String, Value>, key: &str) -> Option<i64> {
    attrs
        .get(key)
        .and_then(|v| v.as_u64())
        .and_then(|n| i64::try_from(n).ok())
}

fn bool_attr(attrs: &Map<String, Value>, key: &str) -> Option<bool> {
    attrs.get(key).and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_timing_events() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = SolveTimingRecorder::new(dir.path());
        recorder
            .append(SolveTimingEvent {
                kind: "turn_started".to_string(),
                ts_ms: 100,
                iteration: None,
                turn_id: None,
                tool_use_id: None,
                tool_name: None,
                duration_ms: None,
                is_error: None,
                output_size: None,
                input_size: None,
                pending_tool_use_count: None,
                input_tokens: None,
                output_tokens: None,
                source: None,
                error: None,
            })
            .unwrap();
        recorder.record_direct_tool("bash", 12, false).unwrap();
        let events = read_solve_timing_events(dir.path(), 10).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "turn_started");
        assert_eq!(events[1].source.as_deref(), Some("direct"));
    }

    #[test]
    fn truncate_clears_timing_file() {
        let dir = tempfile::tempdir().unwrap();
        let recorder = SolveTimingRecorder::new(dir.path());
        recorder
            .append(SolveTimingEvent {
                kind: "turn_started".to_string(),
                ts_ms: 1,
                iteration: None,
                turn_id: None,
                tool_use_id: None,
                tool_name: None,
                duration_ms: None,
                is_error: None,
                output_size: None,
                input_size: None,
                pending_tool_use_count: None,
                input_tokens: None,
                output_tokens: None,
                source: None,
                error: None,
            })
            .unwrap();
        truncate_solve_timing_events(dir.path()).unwrap();
        assert!(read_solve_timing_events(dir.path(), 10).unwrap().is_empty());
    }
}
