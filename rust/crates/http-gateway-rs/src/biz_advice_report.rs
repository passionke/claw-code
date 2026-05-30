//! Boss 报表清洗：网关固定从 `ds_1` 工作区读取 `GPOS_BOSS_REPORT_WRITER` skill 作为润色指令，支持 SSE 流式输出。Author: kejiqing
//!
//! Live solve text is forwarded as-is.

#![allow(
    clippy::must_use_candidate,
    clippy::no_effect_underscore_binding,
    clippy::match_wildcard_for_single_variants
)]

use std::convert::Infallible;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::biz_report_sse_log::{log_sse_delta, log_sse_done, SseDensityAcc};
use axum::response::sse::Event;
use futures_util::stream::{self, Stream, StreamExt as _};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::fs;
use tokio::sync::mpsc;
use tracing::warn;

/// Skill 目录名（`home/skills/<name>/SKILL.md`），可通过 `POST /v1/project/skills/{ds_id}` 维护。
pub const GPOS_BOSS_REPORT_WRITER_SKILL_NAME: &str = "GPOS_BOSS_REPORT_WRITER";

/// 默认润色说明（skill 未部署时的回退，与 crate `skills/gpos-boss-report-writer.SKILL.md` 一致）。Author: kejiqing
pub fn default_gpos_boss_report_writer_skill_md() -> &'static str {
    include_str!("../skills/gpos-boss-report-writer.SKILL.md")
}

/// 去掉 SKILL.md YAML frontmatter，保留正文作为润色指令。
pub fn skill_instructions_for_prompt(skill_content: &str) -> String {
    let trimmed = skill_content.trim();
    if !trimmed.starts_with("---") {
        return trimmed.to_string();
    }
    let mut parts = trimmed.splitn(3, "---");
    let _ = parts.next();
    let _ = parts.next();
    parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

/// Pass-through report text sanitization on external egress.
#[must_use]
pub fn sanitize_external_report_text(text: &str) -> String {
    text.to_string()
}

/// LLM polish stream sanitizer.
#[derive(Debug, Clone, Default)]
pub struct ReportExportSanitizer;

impl ReportExportSanitizer {
    #[must_use]
    pub fn new(_report_section_already_started: bool) -> Self {
        Self
    }

    #[must_use]
    pub fn push_chunk(&mut self, chunk: &str) -> String {
        if chunk.is_empty() {
            return String::new();
        }
        sanitize_external_report_text(chunk)
    }
}

pub fn sanitize_report_payload(payload: &mut BizAdviceReportPayload) {
    if let Some(ref mut text) = payload.report_text {
        *text = sanitize_external_report_text(text);
    }
    if let Some(ref mut json) = payload.report_json {
        sanitize_report_json_value(json);
    }
}

pub fn sanitize_report_json_value(json: &mut Value) {
    if let Some(msg) = json.get("message").and_then(Value::as_str) {
        json["message"] = Value::String(sanitize_external_report_text(msg));
    }
}

/// Non-SSE JSON body (`reportText` + `reportJson.message`).
pub fn sanitize_biz_report_parts(
    report_text: &str,
    report_json: Option<Value>,
) -> (String, Option<Value>) {
    let report_text = sanitize_external_report_text(report_text);
    let report_json = report_json.map(|mut json| {
        sanitize_report_json_value(&mut json);
        json
    });
    (report_text, report_json)
}

/// Solve 报告正文，与 `GET /v1/tasks` → `result.outputJson.message` 同源。Author: kejiqing
pub fn report_body_from_solve_output(
    output_text: &str,
    output_json: Option<&Value>,
) -> Result<String, String> {
    if let Some(json) = output_json {
        if let Some(msg) = json.get("message").and_then(Value::as_str) {
            if !msg.trim().is_empty() {
                return Ok(sanitize_external_report_text(msg));
            }
        }
    }
    let trimmed = output_text.trim();
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            if let Some(msg) = v.get("message").and_then(Value::as_str) {
                if !msg.trim().is_empty() {
                    return Ok(sanitize_external_report_text(msg));
                }
                return Err("solve output has no report message (outputJson.message)".to_string());
            }
        }
    }
    if !trimmed.is_empty() {
        return Ok(sanitize_external_report_text(trimmed));
    }
    Err("solve output has no report message (outputJson.message)".to_string())
}

/// Gateway async failure snapshot (`output_json.detail` from solve 502). Author: kejiqing
pub fn solve_failure_detail_from_output_json(output_json: Option<&Value>) -> Option<String> {
    let detail = output_json?
        .get("detail")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some(sanitize_external_report_text(detail))
}

/// Prefer `output_json.message`, else parse JSON-shaped `report_message` (solve raw output). Author: kejiqing
pub fn report_body_from_persisted(
    report_message: Option<&str>,
    output_json: Option<&Value>,
) -> Option<String> {
    if let Some(raw) = report_message.map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(body) = report_body_from_solve_output(raw, None) {
            return Some(body);
        }
    }
    if let Ok(body) = report_body_from_solve_output("", output_json) {
        return Some(body);
    }
    solve_failure_detail_from_output_json(output_json)
}

pub fn build_biz_advice_polish_prompt(instructions: &str, report_body: &str) -> String {
    format!("{instructions}\n\n【报告正文】\n{report_body}")
}

pub async fn load_boss_report_writer_instructions(work_dir: &Path) -> String {
    let path = work_dir
        .join("home")
        .join("skills")
        .join(GPOS_BOSS_REPORT_WRITER_SKILL_NAME)
        .join("SKILL.md");
    match fs::read_to_string(&path).await {
        Ok(content) => skill_instructions_for_prompt(&content),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                target: "claw_gateway_orchestration",
                component = "biz_advice_report",
                skill = GPOS_BOSS_REPORT_WRITER_SKILL_NAME,
                path = %path.display(),
                "skill not found; using built-in default instructions"
            );
            skill_instructions_for_prompt(default_gpos_boss_report_writer_skill_md())
        }
        Err(e) => {
            warn!(
                target: "claw_gateway_orchestration",
                component = "biz_advice_report",
                skill = GPOS_BOSS_REPORT_WRITER_SKILL_NAME,
                error = %e,
                "read skill failed; using built-in default instructions"
            );
            skill_instructions_for_prompt(default_gpos_boss_report_writer_skill_md())
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BizAdviceReportPayload {
    pub task_id: String,
    pub source_request_id: String,
    pub source_ds_id: i64,
    pub source_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_json: Option<Value>,
}

/// Messages from the in-process polish worker to the HTTP SSE stream.
pub enum BizReportStreamMsg {
    Delta(String),
    Done(BizAdviceReportPayload),
    Error(String),
}

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Split catch-up text into small SSE deltas (avoid one giant snapshot on connect). Author: kejiqing
pub fn split_catchup_chunks(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let max_chars = max_chars.max(1);
    text.chars()
        .collect::<Vec<_>>()
        .chunks(max_chars)
        .map(|chunk| chunk.iter().collect::<String>())
        .filter(|s| !s.is_empty())
        .collect()
}

/// SSE `biz.report.delta` payload (observability for burst / batching). Author: kejiqing
pub fn biz_report_delta_json(
    text: &str,
    seq: u64,
    stream_started_at_ms: u64,
    server_delta_ms: u64,
) -> String {
    let clean = sanitize_external_report_text(text);
    serde_json::json!({
        "text": clean,
        "seq": seq,
        "serverDeltaMs": server_delta_ms,
        "serverTsMs": stream_started_at_ms.saturating_add(server_delta_ms),
        "textLen": clean.len(),
    })
    .to_string()
}

pub fn stream_msg_to_event(msg: &BizReportStreamMsg) -> Event {
    stream_msg_to_event_obs(msg, None)
}

pub fn stream_msg_to_event_obs(msg: &BizReportStreamMsg, obs: Option<(u64, u64, u64)>) -> Event {
    match msg {
        BizReportStreamMsg::Delta(text) => {
            let body = if let Some((seq, stream_started_at_ms, server_delta_ms)) = obs {
                biz_report_delta_json(text, seq, stream_started_at_ms, server_delta_ms)
            } else {
                serde_json::json!({ "text": sanitize_external_report_text(text) }).to_string()
            };
            Event::default().event("biz.report.delta").data(body)
        }
        BizReportStreamMsg::Done(payload) => {
            let mut payload = payload.clone();
            sanitize_report_payload(&mut payload);
            let body = serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string());
            Event::default().event("biz.report.done").data(body)
        }
        BizReportStreamMsg::Error(detail) => Event::default()
            .event("biz.report.error")
            .data(serde_json::json!({ "detail": detail }).to_string()),
    }
}

/// DB terminal snapshot: `start` + single `done` (no delta frames). Author: kejiqing
pub fn db_snapshot_report_sse_response(
    task_id: &str,
    mut payload: BizAdviceReportPayload,
    report_text: &str,
) -> axum::response::Response {
    use axum::http::{header, HeaderValue};
    use axum::response::sse::{KeepAlive, Sse};
    use axum::response::{AppendHeaders, IntoResponse};

    let (tx, rx) = mpsc::unbounded_channel::<BizReportStreamMsg>();
    sanitize_report_payload(&mut payload);
    let _ = tx.send(BizReportStreamMsg::Done(payload));
    drop(tx);

    let no_buffer = header::HeaderName::from_static("x-accel-buffering");
    let no_buffer_val = HeaderValue::from_static("no");
    let _report_text = report_text;
    (
        AppendHeaders([(no_buffer, no_buffer_val)]),
        Sse::new(biz_report_sse_event_stream(task_id, rx)).keep_alive(KeepAlive::default()),
    )
        .into_response()
}

/// Push snapshot text once then `done` (no LLM polish). Author: kejiqing
pub fn enqueue_snapshot_biz_report_sse(
    tx: &mpsc::UnboundedSender<BizReportStreamMsg>,
    mut payload: BizAdviceReportPayload,
    report_text: &str,
) {
    let clean = sanitize_external_report_text(report_text);
    if !clean.is_empty() {
        let _ = tx.send(BizReportStreamMsg::Delta(clean));
    }
    sanitize_report_payload(&mut payload);
    let _ = tx.send(BizReportStreamMsg::Done(payload));
}

/// SSE body: `biz.report.start` then ordered `delta` / `done` (PG catch-up via `delta` only). Author: kejiqing
pub fn biz_report_sse_event_stream(
    task_id: &str,
    rx: mpsc::UnboundedReceiver<BizReportStreamMsg>,
) -> impl Stream<Item = Result<Event, Infallible>> + Send {
    let stream_started_at_ms = wall_clock_ms();
    let start_data = serde_json::json!({
        "taskId": task_id,
        "streamStartedAtMs": stream_started_at_ms,
    })
    .to_string();
    let start = Event::default().event("biz.report.start").data(start_data);
    let t0 = Instant::now();
    let task_id_log = task_id.to_string();
    stream::once(async move { Ok(start) }).chain(stream::unfold(
        (
            rx,
            t0,
            stream_started_at_ms,
            0u64,
            SseDensityAcc::default(),
            task_id_log,
        ),
        |(mut rx, t0, stream_started_at_ms, mut seq, mut acc, task_id)| async move {
            let msg = rx.recv().await?;
            let event = match &msg {
                BizReportStreamMsg::Delta(text) => {
                    seq += 1;
                    let server_delta_ms =
                        u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
                    let clean = sanitize_external_report_text(text);
                    let text_len = u64::try_from(clean.len()).unwrap_or(u64::MAX);
                    acc.on_delta(server_delta_ms, text_len);
                    log_sse_delta(
                        &task_id,
                        seq,
                        server_delta_ms,
                        text_len,
                        acc.same_server_streak(),
                    );
                    stream_msg_to_event_obs(
                        &msg,
                        Some((seq, stream_started_at_ms, server_delta_ms)),
                    )
                }
                BizReportStreamMsg::Done(payload) => {
                    let stream_duration_ms =
                        u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
                    acc.finalize();
                    log_sse_done(&task_id, &acc, stream_duration_ms);
                    let mut payload = payload.clone();
                    sanitize_report_payload(&mut payload);
                    let mut v = serde_json::to_value(&payload).unwrap_or_else(|_| json!({}));
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("deltaCount".into(), json!(seq));
                        obj.insert("streamDurationMs".into(), json!(stream_duration_ms));
                        obj.insert("maxBucketCount1ms".into(), json!(acc.max_bucket_1ms()));
                        obj.insert(
                            "maxSameServerStreak".into(),
                            json!(acc.max_same_server_streak),
                        );
                    }
                    let body = serde_json::to_string(&v).unwrap_or_else(|_| "{}".to_string());
                    Event::default().event("biz.report.done").data(body)
                }
                _ => stream_msg_to_event_obs(&msg, None),
            };
            tokio::task::yield_now().await;
            Some((Ok(event), (rx, t0, stream_started_at_ms, seq, acc, task_id)))
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_skill_frontmatter() {
        let md = "---\nname: x\n---\n\nBody line\n";
        assert_eq!(skill_instructions_for_prompt(md), "Body line");
    }

    #[test]
    fn build_prompt_includes_report_body_section() {
        let p = build_biz_advice_polish_prompt("instr", "report text");
        assert!(p.contains("instr"));
        assert!(p.contains("【报告正文】"));
        assert!(p.contains("report text"));
    }

    #[test]
    fn report_body_prefers_output_json_message() {
        let json = serde_json::json!({"message": "body", "iterations": 1});
        assert_eq!(
            report_body_from_solve_output("", Some(&json)).unwrap(),
            "body"
        );
    }

    #[test]
    fn report_body_from_persisted_uses_failure_detail() {
        let json = serde_json::json!({
            "detail": "api returned 404 Not Found",
            "status_code": 502
        });
        assert_eq!(
            report_body_from_persisted(None, Some(&json)).as_deref(),
            Some("api returned 404 Not Found")
        );
    }

    #[test]
    fn report_body_from_persisted_parses_json_report_message() {
        let raw = "{\"iterations\":1,\"message\":\"## 标题\\n正文\",\"model\":\"m\"}";
        assert_eq!(
            report_body_from_persisted(Some(raw), None).as_deref(),
            Some("## 标题\n正文")
        );
    }

    #[test]
    fn report_body_rejects_solve_envelope_with_empty_message() {
        let raw = r#"{"iterations":2,"message":"","model":"qwen3.7-max"}"#;
        assert!(report_body_from_solve_output(raw, None).is_err());
        assert!(report_body_from_persisted(Some(raw), None).is_none());
    }

    #[test]
    fn sanitizer_passes_through_without_marker() {
        let mut s = ReportExportSanitizer::new(false);
        assert_eq!(s.push_chunk("分析中…"), "分析中…");
        assert_eq!(s.push_chunk("润色正文"), "润色正文");
    }

    #[test]
    fn sanitize_parts_passes_through_text() {
        let (text, json) =
            sanitize_biz_report_parts("# 标题", Some(serde_json::json!({ "message": "正文" })));
        assert_eq!(text, "# 标题");
        assert_eq!(json.unwrap()["message"].as_str().unwrap(), "正文");
        let mut payload = BizAdviceReportPayload {
            task_id: "t".into(),
            source_request_id: "t".into(),
            source_ds_id: 1,
            source_status: "succeeded".into(),
            report_text: Some("delta".into()),
            report_json: None,
        };
        sanitize_report_payload(&mut payload);
        assert_eq!(payload.report_text.as_deref(), Some("delta"));
    }

    #[test]
    fn report_body_reads_plain_message() {
        let json = serde_json::json!({
            "message": "# 报告\n正文"
        });
        assert_eq!(
            report_body_from_solve_output("", Some(&json)).unwrap(),
            "# 报告\n正文"
        );
    }

    #[test]
    fn delta_json_includes_observability_fields() {
        let raw = biz_report_delta_json("ab", 3, 1_700_000_000_000, 42);
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["text"].as_str(), Some("ab"));
        assert_eq!(v["seq"].as_u64(), Some(3));
        assert_eq!(v["serverDeltaMs"].as_u64(), Some(42));
        assert_eq!(v["serverTsMs"].as_u64(), Some(1_700_000_000_042));
        assert_eq!(v["textLen"].as_u64(), Some(2));
    }

    #[test]
    fn split_catchup_chunks_respects_char_boundary() {
        let parts = split_catchup_chunks("一二三四五", 2);
        assert_eq!(parts, vec!["一二", "三四", "五"]);
    }
}
