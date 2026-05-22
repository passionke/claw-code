//! In-memory boundary mocks for live-report tests (no PostgreSQL). Author: kejiqing

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use http_gateway_rs::session_db::LiveChunkRow;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::live_report_ports::{AssistantStreamStore, LiveReportPort};

#[derive(Clone, Default)]
pub struct MockIngestStore {
    turns: Arc<Mutex<Vec<String>>>,
    chunks: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl MockIngestStore {
    pub async fn register_turn(&self, turn_id: &str) {
        self.turns.lock().await.push(turn_id.to_string());
    }

    pub async fn chunks_for(&self, turn_id: &str) -> Vec<String> {
        self.chunks
            .lock()
            .await
            .get(turn_id)
            .cloned()
            .unwrap_or_default()
    }
}

#[async_trait]
impl AssistantStreamStore for MockIngestStore {
    async fn has_turn(&self, turn_id: &str) -> Result<bool, String> {
        Ok(self.turns.lock().await.iter().any(|t| t == turn_id))
    }

    async fn append_live_chunks(
        &self,
        turn_id: &str,
        chunks: &[String],
        _created_at_ms: i64,
    ) -> Result<(), String> {
        let mut map = self.chunks.lock().await;
        map.entry(turn_id.to_string())
            .or_default()
            .extend(chunks.iter().cloned());
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct MockLiveReportPort {
    pub status: Arc<Mutex<Option<String>>>,
    pub chunks: Arc<Mutex<Vec<LiveChunkRow>>>,
    pub report_message: Arc<Mutex<Option<String>>>,
    pub output_json: Arc<Mutex<Option<Value>>>,
}

impl MockLiveReportPort {
    pub async fn push_chunk(&self, turn_id: &str, seq: i64, chunk: &str) {
        self.chunks.lock().await.push(LiveChunkRow {
            seq,
            chunk: chunk.to_string(),
            created_at_ms: 0,
        });
        let _ = turn_id;
    }

    pub async fn set_succeeded(&self, report: &str) {
        *self.status.lock().await = Some("succeeded".into());
        *self.report_message.lock().await = Some(report.into());
    }
}

#[async_trait]
impl LiveReportPort for MockLiveReportPort {
    async fn turn_status(
        &self,
        _turn_id: &str,
        _session_id: &str,
        _ds_id: i64,
    ) -> Result<Option<String>, String> {
        Ok(self.status.lock().await.clone())
    }

    async fn stream_live_chunks_since(
        &self,
        _turn_id: &str,
        after_seq: i64,
    ) -> Result<Vec<LiveChunkRow>, String> {
        Ok(self
            .chunks
            .lock()
            .await
            .iter()
            .filter(|r| r.seq > after_seq)
            .cloned()
            .collect())
    }

    async fn formal_report_text(
        &self,
        _turn_id: &str,
        _session_id: &str,
        _ds_id: i64,
    ) -> Result<Option<String>, String> {
        let report_message = self.report_message.lock().await.clone();
        let output_json = self.output_json.lock().await.clone();
        Ok(crate::biz_advice_report_live::formal_report_text_from_db_snapshot(
            report_message.as_deref(),
            output_json.as_ref(),
        ))
    }
}
