//! Structured logging for biz report SSE density (tracing + optional file). Author: kejiqing

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::{Mutex, OnceLock};

use serde_json::json;

static SSE_FILE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn file_lock() -> &'static Mutex<()> {
    SSE_FILE_LOCK.get_or_init(|| Mutex::new(()))
}

fn sse_log_path() -> Option<String> {
    std::env::var("CLAW_SSE_LOG_FILE")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

#[derive(Debug, Default)]
pub struct SseDensityAcc {
    pub delta_count: u64,
    pub chars_total: u64,
    pub text_len_max: u64,
    pub large_ge200: u64,
    last_server_ms: Option<u64>,
    same_server_streak: u32,
    pub max_same_server_streak: u32,
    pub max_same_server_at_ms: u64,
    server_bucket_1ms: HashMap<u64, u32>,
}

impl SseDensityAcc {
    pub fn on_delta(&mut self, server_delta_ms: u64, text_len: u64) {
        self.delta_count += 1;
        self.chars_total += text_len;
        if text_len > self.text_len_max {
            self.text_len_max = text_len;
        }
        if text_len >= 200 {
            self.large_ge200 += 1;
        }
        *self
            .server_bucket_1ms
            .entry(server_delta_ms)
            .or_insert(0) += 1;
        if self.last_server_ms == Some(server_delta_ms) {
            self.same_server_streak += 1;
        } else {
            if self.same_server_streak + 1 > self.max_same_server_streak {
                self.max_same_server_streak = self.same_server_streak + 1;
                self.max_same_server_at_ms = self.last_server_ms.unwrap_or(server_delta_ms);
            }
            self.same_server_streak = 0;
            self.last_server_ms = Some(server_delta_ms);
        }
    }

    pub fn same_server_streak(&self) -> u32 {
        self.same_server_streak
    }

    pub fn finalize(&mut self) {
        if self.same_server_streak + 1 > self.max_same_server_streak {
            self.max_same_server_streak = self.same_server_streak + 1;
            self.max_same_server_at_ms = self.last_server_ms.unwrap_or(0);
        }
    }

    pub fn max_bucket_1ms(&self) -> u32 {
        self.server_bucket_1ms.values().copied().max().unwrap_or(0)
    }

    pub fn hot_buckets(&self, top: usize) -> Vec<(u64, u32)> {
        let mut v: Vec<_> = self
            .server_bucket_1ms
            .iter()
            .map(|(k, c)| (*k, *c))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.truncate(top);
        v
    }
}

fn append_file(line: &str) {
    let Some(path) = sse_log_path() else {
        return;
    };
    let Ok(_guard) = file_lock().lock() else {
        return;
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

pub fn log_sse_delta(
    task_id: &str,
    seq: u64,
    server_delta_ms: u64,
    text_len: u64,
    same_server_streak: u32,
) {
    let burst = same_server_streak >= 4;
    tracing::info!(
        target: "biz_report_sse",
        component = "biz_advice_report",
        task_id = %task_id,
        seq = seq,
        server_delta_ms = server_delta_ms,
        text_len = text_len,
        same_server_streak = same_server_streak,
        burst = burst,
        "biz.report.delta"
    );
    if burst {
        tracing::warn!(
            target: "biz_report_sse",
            component = "biz_advice_report",
            task_id = %task_id,
            server_delta_ms = server_delta_ms,
            same_server_streak = same_server_streak,
            "biz.report.server_burst"
        );
    }
    let row = json!({
        "ev": "biz.report.delta",
        "taskId": task_id,
        "seq": seq,
        "serverDeltaMs": server_delta_ms,
        "textLen": text_len,
        "sameServerStreak": same_server_streak,
    });
    append_file(&row.to_string());
}

pub fn log_sse_done(task_id: &str, acc: &SseDensityAcc, stream_duration_ms: u64) {
    let hot: Vec<_> = acc
        .hot_buckets(5)
        .into_iter()
        .map(|(ms, c)| json!({"serverDeltaMs": ms, "count": c}))
        .collect();
    tracing::info!(
        target: "biz_report_sse",
        component = "biz_advice_report",
        task_id = %task_id,
        delta_count = acc.delta_count,
        stream_duration_ms = stream_duration_ms,
        chars_total = acc.chars_total,
        text_len_max = acc.text_len_max,
        large_delta_ge200 = acc.large_ge200,
        max_bucket_count_1ms = acc.max_bucket_1ms(),
        max_same_server_streak = acc.max_same_server_streak,
        max_same_server_at_ms = acc.max_same_server_at_ms,
        hot_buckets = %serde_json::to_string(&hot).unwrap_or_else(|_| "[]".into()),
        "biz.report.stream_done"
    );
    let row = json!({
        "ev": "biz.report.stream_done",
        "taskId": task_id,
        "deltaCount": acc.delta_count,
        "streamDurationMs": stream_duration_ms,
        "charsTotal": acc.chars_total,
        "textLenMax": acc.text_len_max,
        "largeDeltaGe200": acc.large_ge200,
        "maxBucketCount1ms": acc.max_bucket_1ms(),
        "maxSameServerStreak": acc.max_same_server_streak,
        "maxSameServerAtMs": acc.max_same_server_at_ms,
        "hotBuckets1ms": hot,
    });
    append_file(&row.to_string());
}

pub fn log_stdout_ingest(turn_id: &str, text_len: usize) {
    tracing::info!(
        target: "biz_report_sse",
        component = "turn_stdout_hub",
        turn_id = %turn_id,
        text_len = text_len,
        "report.delta.ingest"
    );
    let row = json!({
        "ev": "report.delta.ingest",
        "turnId": turn_id,
        "textLen": text_len,
    });
    append_file(&row.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acc_tracks_same_ms_streak() {
        let mut a = SseDensityAcc::default();
        a.on_delta(10, 2);
        a.on_delta(10, 3);
        a.on_delta(10, 1);
        a.on_delta(11, 5);
        a.finalize();
        assert_eq!(a.delta_count, 4);
        assert_eq!(a.max_same_server_streak, 3);
        assert_eq!(a.max_bucket_1ms(), 3);
    }
}
