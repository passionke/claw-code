//! Append-only orchestration events for ProgressNarrator. Author: kejiqing

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::multi_agent::plan::AnalysisPlan;

pub const ORCHESTRATION_EVENTS_REL: &str = ".claw/orchestration-events.ndjson";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationEvent {
    pub kind: String,
    #[serde(rename = "tsMs")]
    pub ts_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub todo_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<AnalysisPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct EventBus {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl EventBus {
    pub fn new(session_home: &Path) -> Self {
        Self {
            path: session_home.join(ORCHESTRATION_EVENTS_REL),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    pub fn append(&self, event: OrchestrationEvent) -> Result<(), String> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| String::from("event bus lock poisoned"))?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create event dir: {e}"))?;
        }
        let line = serde_json::to_string(&event).map_err(|e| format!("serialize event: {e}"))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("open events file: {e}"))?;
        writeln!(file, "{line}").map_err(|e| format!("write event: {e}"))?;
        Ok(())
    }

    pub fn emit(&self, kind: &str, extra: Value) -> Result<(), String> {
        let mut obj = extra.as_object().cloned().unwrap_or_default();
        obj.insert(String::from("kind"), Value::String(kind.to_string()));
        obj.insert(String::from("tsMs"), json!(Self::now_ms()));
        let event: OrchestrationEvent =
            serde_json::from_value(Value::Object(obj)).map_err(|e| format!("build event: {e}"))?;
        self.append(event)
    }

    pub fn session_started(&self) -> Result<(), String> {
        self.emit("session_started", json!({}))
    }

    pub fn preflight_done(&self) -> Result<(), String> {
        self.emit("preflight_done", json!({}))
    }

    pub fn plan_ready(&self, plan: &AnalysisPlan) -> Result<(), String> {
        self.emit(
            "plan_ready",
            json!({
                "plan": plan,
                "message": plan.plan_title.clone(),
            }),
        )
    }

    pub fn query_started(&self, todo_id: &str, title: &str) -> Result<(), String> {
        self.emit(
            "query_started",
            json!({
                "todoId": todo_id,
                "message": title,
            }),
        )
    }

    pub fn query_done(&self, todo_id: &str, duration_ms: i64) -> Result<(), String> {
        self.emit(
            "query_done",
            json!({
                "todoId": todo_id,
                "durationMs": duration_ms,
            }),
        )
    }

    pub fn query_failed(&self, todo_id: &str, error: &str) -> Result<(), String> {
        self.emit(
            "query_failed",
            json!({
                "todoId": todo_id,
                "error": error,
            }),
        )
    }

    pub fn writer_started(&self) -> Result<(), String> {
        self.emit("writer_started", json!({}))
    }

    pub fn writer_done(&self) -> Result<(), String> {
        self.emit("writer_done", json!({}))
    }

    /// Read all events (for narrator batch).
    pub fn read_all(&self) -> Result<Vec<OrchestrationEvent>, String> {
        if !self.path.is_file() {
            return Ok(Vec::new());
        }
        let raw = fs::read_to_string(&self.path).map_err(|e| format!("read events: {e}"))?;
        let mut out = Vec::new();
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(ev) = serde_json::from_str::<OrchestrationEvent>(line) {
                out.push(ev);
            }
        }
        Ok(out)
    }

    #[allow(dead_code)]
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}
