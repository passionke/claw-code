//! NDJSON burst-localization trace (opt-in via `CLAW_SSE_BURST_TRACE`). Author: kejiqing
//!
//! Correlates HTTP read chunks → text deltas → worker stdout → pool ingest.
//! No behavior change when disabled.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

const BURST_TRACE_ENV: &str = "CLAW_SSE_BURST_TRACE";
const BURST_LOG_FILE_ENV: &str = "CLAW_SSE_BURST_LOG_FILE";
const DEFAULT_BURST_LOG_PATH: &str = "/var/log/claw/sse-burst-trace.ndjson";

static LOG_FILE: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
static START_INSTANT: OnceLock<Instant> = OnceLock::new();
static EMIT_SEQ: AtomicU64 = AtomicU64::new(0);
static INGEST_SEQ: AtomicU64 = AtomicU64::new(0);

/// Per-stream counters set by `MessageStream` (HTTP chunk boundaries).
#[derive(Debug, Default)]
pub struct BurstStreamCtx {
    pub raw_chunk: u64,
    pub delta_in_chunk: u32,
}

impl BurstStreamCtx {
    pub fn on_http_chunk(&mut self, bytes: usize) {
        self.raw_chunk = self.raw_chunk.saturating_add(1);
        self.delta_in_chunk = 0;
        log_event(
            "http_chunk",
            json!({
                "rawChunk": self.raw_chunk,
                "bytes": bytes,
            }),
        );
    }

    pub fn on_text_delta(&mut self, text_len: usize) {
        self.delta_in_chunk = self.delta_in_chunk.saturating_add(1);
        log_event(
            "text_delta",
            json!({
                "rawChunk": self.raw_chunk,
                "deltaInChunk": self.delta_in_chunk,
                "textLen": text_len,
            }),
        );
    }
}

#[must_use]
pub fn burst_trace_enabled() -> bool {
    std::env::var(BURST_TRACE_ENV).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

pub fn log_worker_emit(text_len: usize) {
    if !burst_trace_enabled() {
        return;
    }
    let seq = EMIT_SEQ.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    log_event(
        "worker_emit",
        json!({
            "emitSeq": seq,
            "textLen": text_len,
        }),
    );
}

#[derive(Debug, Default)]
struct TurnIngestBatch {
    last_mono_ms: u64,
    batch_id: u64,
    lines_in_batch: u32,
}

static INGEST_BATCH: OnceLock<Mutex<HashMap<String, TurnIngestBatch>>> = OnceLock::new();

pub fn log_pool_ingest(turn_id: &str, text_len: usize) {
    if !burst_trace_enabled() {
        return;
    }
    let mono = mono_ms();
    let (batch_id, lines_in_batch) = {
        let guard = INGEST_BATCH.get_or_init(|| Mutex::new(HashMap::new()));
        let Ok(mut map) = guard.lock() else {
            return;
        };
        let state = map.entry(turn_id.to_string()).or_default();
        if state.last_mono_ms == mono {
            state.lines_in_batch = state.lines_in_batch.saturating_add(1);
        } else {
            state.batch_id = state.batch_id.saturating_add(1);
            state.lines_in_batch = 1;
            state.last_mono_ms = mono;
        }
        (state.batch_id, state.lines_in_batch)
    };
    let seq = INGEST_SEQ.fetch_add(1, Ordering::Relaxed).saturating_add(1);
    log_event(
        "pool_ingest",
        json!({
            "turnId": turn_id,
            "ingestSeq": seq,
            "textLen": text_len,
            "readerBatchId": batch_id,
            "linesInBatch": lines_in_batch,
        }),
    );
}

pub fn log_event(ev: &str, mut fields: serde_json::Value) {
    if !burst_trace_enabled() {
        return;
    }
    let mono_ms = mono_ms();
    let wall_ms = wall_ms();
    let obj = fields.as_object_mut();
    if let Some(map) = obj {
        map.insert("ev".into(), json!(ev));
        map.insert("monoMs".into(), json!(mono_ms));
        map.insert("wallMs".into(), json!(wall_ms));
    }
    let line = fields.to_string();
    append_line(&line);
}

fn mono_ms() -> u64 {
    let start = START_INSTANT.get_or_init(Instant::now);
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn log_path() -> String {
    std::env::var(BURST_LOG_FILE_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BURST_LOG_PATH.to_string())
}

fn append_line(line: &str) {
    let mutex = LOG_FILE.get_or_init(|| Mutex::new(None));
    let Ok(mut guard) = mutex.lock() else {
        return;
    };
    if guard.is_none() {
        let path = log_path();
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        *guard = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
    }
    if let Some(file) = guard.as_mut() {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}
