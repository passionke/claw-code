//! POST assistant-stream NDJSON to the gateway (`CLAW_GATEWAY_INTERNAL_*`). Author: kejiqing

use std::io::{self, Read};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::thread;
use std::time::Duration;

use serde_json::json;

/// Background upload of model text deltas to `POST /v1/internal/turns/{turnId}/assistant-stream`.
pub struct TurnOutputStreamClient {
    line_tx: SyncSender<String>,
    join: Option<thread::JoinHandle<()>>,
}

impl TurnOutputStreamClient {
    /// When `CLAW_GATEWAY_INTERNAL_BASE_URL` and `CLAW_GATEWAY_INTERNAL_TOKEN` are set. Author: kejiqing
    #[must_use]
    pub fn try_new(turn_id: &str) -> Option<Self> {
        let base = std::env::var("CLAW_GATEWAY_INTERNAL_BASE_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let token = std::env::var("CLAW_GATEWAY_INTERNAL_TOKEN")
            .ok()
            .filter(|s| !s.trim().is_empty())?;
        let base = base.trim_end_matches('/').to_string();
        let url = format!("{base}/v1/internal/turns/{turn_id}/assistant-stream");
        let (line_tx, line_rx) = sync_channel::<String>(512);
        let join = thread::spawn(move || {
            if let Err(e) = upload_ndjson_stream(&url, &token, line_rx) {
                eprintln!("turn_output_stream: upload failed: {e}");
            }
        });
        Some(Self {
            line_tx,
            join: Some(join),
        })
    }

    pub fn push_text_delta(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let _ = self.line_tx.send(assistant_stream_ndjson_line(text));
    }
}

/// One NDJSON line for `POST .../assistant-stream` (gateway ingest contract). Author: kejiqing
#[must_use]
pub fn assistant_stream_ndjson_line(text: &str) -> String {
    json!({ "chunk": text }).to_string() + "\n"
}

impl Drop for TurnOutputStreamClient {
    fn drop(&mut self) {
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

struct NdjsonLineReader {
    rx: Receiver<String>,
    pending: Option<Vec<u8>>,
    pos: usize,
}

impl Read for NdjsonLineReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(ref pending) = self.pending {
                let avail = &pending[self.pos..];
                if !avail.is_empty() {
                    let n = avail.len().min(buf.len());
                    buf[..n].copy_from_slice(&avail[..n]);
                    self.pos += n;
                    if self.pos >= pending.len() {
                        self.pending = None;
                        self.pos = 0;
                    }
                    return Ok(n);
                }
                self.pending = None;
                self.pos = 0;
            }
            match self.rx.recv() {
                Ok(line) => {
                    self.pending = Some(line.into_bytes());
                    self.pos = 0;
                }
                Err(_) => return Ok(0),
            }
        }
    }
}

fn upload_ndjson_stream(
    url: &str,
    token: &str,
    line_rx: Receiver<String>,
) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let body = reqwest::blocking::Body::new(NdjsonLineReader {
        rx: line_rx,
        pending: None,
        pos: 0,
    });
    let resp = client
        .post(url)
        .header("x-claw-gateway-internal-token", token)
        .header("content-type", "application/x-ndjson")
        .body(body)
        .send()
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("assistant-stream HTTP {}", resp.status()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_stream_ndjson_line_shape() {
        let line = assistant_stream_ndjson_line("wire-Δ");
        assert!(line.ends_with('\n'));
        assert!(line.contains(r#""chunk":"wire-Δ""#));
    }

    #[test]
    fn try_new_absent_without_env() {
        let prev_base = std::env::var("CLAW_GATEWAY_INTERNAL_BASE_URL").ok();
        let prev_tok = std::env::var("CLAW_GATEWAY_INTERNAL_TOKEN").ok();
        std::env::remove_var("CLAW_GATEWAY_INTERNAL_BASE_URL");
        std::env::remove_var("CLAW_GATEWAY_INTERNAL_TOKEN");
        assert!(TurnOutputStreamClient::try_new("T_10000000000000000000000000000001").is_none());
        if let Some(v) = prev_base {
            std::env::set_var("CLAW_GATEWAY_INTERNAL_BASE_URL", v);
        }
        if let Some(v) = prev_tok {
            std::env::set_var("CLAW_GATEWAY_INTERNAL_TOKEN", v);
        }
    }
}
