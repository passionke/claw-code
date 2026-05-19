//! Optional per-iteration assistant text spill while the model streams (left on disk for live report tail).
//! Author: kejiqing

use runtime::GATEWAY_LIVE_REPORT_START_MARKER;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Filename prefix under `<session>/.claw/`; actual name is `assistant-stream-spill-{turnId}.txt`.
pub const ASSISTANT_STREAM_SPILL_BASENAME_PREFIX: &str = "assistant-stream-spill-";

/// Written at turn end (`mark_turn_stream_complete`); live report SSE switches to session jsonl.
pub const ASSISTANT_STREAM_SPILL_END_MARKER: &str = "__CLAW_ASSISTANT_STREAM_END__";

/// Same token as system-prompt instruction (`runtime::GATEWAY_LIVE_REPORT_START_MARKER`).
pub const ASSISTANT_STREAM_REPORT_START_MARKER: &str = GATEWAY_LIVE_REPORT_START_MARKER;

#[must_use]
pub fn assistant_stream_spill_path(session_home: &Path, turn_id: &str) -> PathBuf {
    session_home.join(".claw").join(format!(
        "{ASSISTANT_STREAM_SPILL_BASENAME_PREFIX}{turn_id}.txt"
    ))
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|v| {
        let s = v.trim().to_ascii_lowercase();
        matches!(s.as_str(), "1" | "true" | "yes" | "on")
    })
}

/// Worker-side default when the task file omits `assistantStreamSpill`.
#[must_use]
pub fn assistant_stream_spill_enabled_from_env() -> bool {
    env_flag("CLAW_GATEWAY_ASSISTANT_STREAM_SPILL")
}

/// Resolve spill for `gateway-solve-once` (task JSON overrides env).
#[must_use]
pub fn resolve_assistant_stream_spill(task_flag: Option<bool>) -> bool {
    task_flag.unwrap_or_else(assistant_stream_spill_enabled_from_env)
}

/// Incremental UTF-8 spill for one assistant iteration (one provider `stream()` call).
#[derive(Debug)]
pub struct AssistantStreamSpill {
    path: PathBuf,
}

impl AssistantStreamSpill {
    #[must_use]
    pub fn new(session_home: &Path, turn_id: &str) -> Self {
        Self {
            path: assistant_stream_spill_path(session_home, turn_id),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Truncate this turn's spill at the start of each assistant `stream()` inside one solve
    /// (tool loop may call the model multiple times; same `turnId` file is reused).
    pub fn begin_iteration(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create spill dir failed: {e}"))?;
        }
        fs::write(&self.path, []).map_err(|e| format!("truncate spill failed: {e}"))
    }

    pub fn append(&self, delta: &str) -> Result<(), String> {
        if delta.is_empty() {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create spill dir failed: {e}"))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("open spill failed: {e}"))?;
        file.write_all(delta.as_bytes())
            .map_err(|e| format!("write spill failed: {e}"))?;
        file.flush().map_err(|e| format!("flush spill failed: {e}"))
    }

    /// Append end marker so live-report SSE can switch to formal jsonl (spill file is not deleted).
    pub fn mark_turn_stream_complete(session_home: &Path, turn_id: &str) -> Result<(), String> {
        let spill = Self::new(session_home, turn_id);
        if let Some(parent) = spill.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create spill dir failed: {e}"))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&spill.path)
            .map_err(|e| format!("open spill for end marker failed: {e}"))?;
        writeln!(file, "{ASSISTANT_STREAM_SPILL_END_MARKER}")
            .map_err(|e| format!("write spill end marker failed: {e}"))?;
        file.flush()
            .map_err(|e| format!("flush spill end marker failed: {e}"))
    }
}

/// Split spill bytes into `(visible_text, saw_end_marker)`.
#[must_use]
pub fn split_spill_end_marker(content: &str) -> (String, bool) {
    if let Some(idx) = content.find(ASSISTANT_STREAM_SPILL_END_MARKER) {
        return (content[..idx].to_string(), true);
    }
    (content.to_string(), false)
}

/// Whether this turn's spill file exists and contains [`ASSISTANT_STREAM_REPORT_START_MARKER`].
#[must_use]
pub fn spill_contains_report_start_marker(session_home: &Path, turn_id: &str) -> bool {
    let path = assistant_stream_spill_path(session_home, turn_id);
    let Ok(contents) = fs::read_to_string(&path) else {
        return false;
    };
    contents.contains(ASSISTANT_STREAM_REPORT_START_MARKER)
}

#[must_use]
pub fn spill_bytes_contain_end_marker(content: &[u8]) -> bool {
    content
        .windows(ASSISTANT_STREAM_SPILL_END_MARKER.len())
        .any(|w| w == ASSISTANT_STREAM_SPILL_END_MARKER.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spill_append_and_end_marker_keeps_file() {
        let dir = std::env::temp_dir().join(format!(
            "claw-spill-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let _ = fs::create_dir_all(&dir);
        let turn_id = "T_a1b2c3d4e5f6478990abcdef12345678";
        let spill = AssistantStreamSpill::new(&dir, turn_id);
        spill.begin_iteration().unwrap();
        spill.append("hel").unwrap();
        spill.append("lo").unwrap();
        AssistantStreamSpill::mark_turn_stream_complete(Path::new(&dir), turn_id).unwrap();
        let raw = fs::read_to_string(spill.path()).unwrap();
        assert!(raw.starts_with("hello"));
        assert!(raw.contains(ASSISTANT_STREAM_SPILL_END_MARKER));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_report_start_marker_in_spill() {
        let dir = std::env::temp_dir().join(format!(
            "claw-spill-rs-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let turn_id = "T_a1b2c3d4e5f6478990abcdef12345678";
        let spill = AssistantStreamSpill::new(Path::new(&dir), turn_id);
        spill.begin_iteration().unwrap();
        assert!(!spill_contains_report_start_marker(
            Path::new(&dir),
            turn_id
        ));
        spill
            .append(&format!(
                "分析中…\n{ASSISTANT_STREAM_REPORT_START_MARKER}\n# 报告\n"
            ))
            .unwrap();
        assert!(spill_contains_report_start_marker(Path::new(&dir), turn_id));
        let _ = fs::remove_dir_all(&dir);
    }
}
