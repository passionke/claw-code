//! Ports at live-report system boundaries (ingest + SSE); production uses PG adapters. Author: kejiqing

use std::sync::Arc;

use async_trait::async_trait;
use http_gateway_rs::session_db::{GatewaySessionDb, LiveChunkRow};

/// Worker → gateway ingest boundary (turn lookup + chunk persistence). Author: kejiqing
#[async_trait]
pub trait AssistantStreamStore: Send + Sync {
    async fn has_turn(&self, turn_id: &str) -> Result<bool, String>;
    async fn append_live_chunks(
        &self,
        turn_id: &str,
        chunks: &[String],
        created_at_ms: i64,
    ) -> Result<(), String>;
}

/// Gateway SSE worker ↔ turn snapshot + live chunks boundary. Author: kejiqing
#[async_trait]
pub trait LiveReportPort: Send + Sync {
    async fn turn_status(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, String>;
    async fn stream_live_chunks_since(
        &self,
        turn_id: &str,
        after_seq: i64,
    ) -> Result<Vec<LiveChunkRow>, String>;
    /// `None` = formal report not ready yet (non-terminal or empty snapshot).
    async fn formal_report_text(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, String>;
}

/// [`GatewaySessionDb`] adapter for ingest. Author: kejiqing
pub struct SessionDbIngestAdapter(pub Arc<GatewaySessionDb>);

#[async_trait]
impl AssistantStreamStore for SessionDbIngestAdapter {
    async fn has_turn(&self, turn_id: &str) -> Result<bool, String> {
        self.0
            .turn_exists(turn_id)
            .await
            .map_err(|e| e.to_string())
    }

    async fn append_live_chunks(
        &self,
        turn_id: &str,
        chunks: &[String],
        created_at_ms: i64,
    ) -> Result<(), String> {
        self.0
            .append_live_chunks(turn_id, chunks, created_at_ms)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// [`GatewaySessionDb`] adapter for live report SSE. Author: kejiqing
pub struct SessionDbLiveReportAdapter(pub Arc<GatewaySessionDb>);

#[async_trait]
impl LiveReportPort for SessionDbLiveReportAdapter {
    async fn turn_status(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, String> {
        self.0
            .get_turn_status(turn_id, session_id, ds_id)
            .await
            .map_err(|e| e.to_string())
    }

    async fn stream_live_chunks_since(
        &self,
        turn_id: &str,
        after_seq: i64,
    ) -> Result<Vec<LiveChunkRow>, String> {
        self.0
            .stream_live_chunks_since(turn_id, after_seq)
            .await
            .map_err(|e| e.to_string())
    }

    async fn formal_report_text(
        &self,
        turn_id: &str,
        session_id: &str,
        ds_id: i64,
    ) -> Result<Option<String>, String> {
        let report_message = self
            .0
            .get_turn_report_message(turn_id, session_id, ds_id)
            .await
            .map_err(|e| e.to_string())?;
        let output_json = if report_message
            .as_ref()
            .is_some_and(|t| !t.trim().is_empty())
        {
            None
        } else {
            self.0
                .get_turn_output_json(turn_id, session_id, ds_id)
                .await
                .map_err(|e| e.to_string())?
        };
        Ok(crate::biz_advice_report_live::formal_report_text_from_db_snapshot(
            report_message.as_deref(),
            output_json.as_ref(),
        ))
    }
}
