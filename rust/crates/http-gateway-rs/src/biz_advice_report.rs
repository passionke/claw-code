//! Boss 报表清洗：网关固定从 `ds_1` 工作区读取 `GPOS_BOSS_REPORT_WRITER` skill 作为润色指令，支持 SSE 流式输出。Author: kejiqing

use std::convert::Infallible;
use std::path::Path;

use axum::response::sse::Event;
use futures_util::stream::{self, Stream, StreamExt as _};
use serde::Serialize;
use serde_json::Value;
use tokio::fs;
use tokio::sync::mpsc;
use tracing::warn;

/// Skill 目录名（`home/skills/<name>/SKILL.md`），可通过 `POST /v1/project/skills/{ds_id}` 维护。
pub const GPOS_BOSS_REPORT_WRITER_SKILL_NAME: &str = "GPOS_BOSS_REPORT_WRITER";

/// 默认润色说明（skill 未部署时的回退，与 `rust/scripts/gpos-boss-report-writer.SKILL.md` 一致）。Author: kejiqing
pub fn default_gpos_boss_report_writer_skill_md() -> &'static str {
    include_str!("../../../scripts/gpos-boss-report-writer.SKILL.md")
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

/// Solve 报告正文，与 `GET /v1/tasks` → `result.outputJson.message` 同源。Author: kejiqing
pub fn report_body_from_solve_output(
    output_text: &str,
    output_json: Option<&Value>,
) -> Result<String, String> {
    if let Some(json) = output_json {
        if let Some(msg) = json.get("message").and_then(Value::as_str) {
            if !msg.trim().is_empty() {
                return Ok(msg.to_string());
            }
        }
    }
    let trimmed = output_text.trim();
    if trimmed.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            if let Some(msg) = v.get("message").and_then(Value::as_str) {
                if !msg.trim().is_empty() {
                    return Ok(msg.to_string());
                }
            }
        }
    }
    if !trimmed.is_empty() {
        return Ok(trimmed.to_string());
    }
    Err("solve output has no report message (outputJson.message)".to_string())
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

pub fn stream_msg_to_event(msg: &BizReportStreamMsg) -> Event {
    match msg {
        BizReportStreamMsg::Delta(text) => Event::default()
            .event("biz.report.delta")
            .data(serde_json::json!({ "text": text }).to_string()),
        BizReportStreamMsg::Done(payload) => {
            let body = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
            Event::default().event("biz.report.done").data(body)
        }
        BizReportStreamMsg::Error(detail) => Event::default()
            .event("biz.report.error")
            .data(serde_json::json!({ "detail": detail }).to_string()),
    }
}

/// SSE body: `start` then channel messages; yields between frames so hyper can flush.
pub fn biz_report_sse_event_stream(
    task_id: &str,
    rx: mpsc::UnboundedReceiver<BizReportStreamMsg>,
) -> impl Stream<Item = Result<Event, Infallible>> + Send {
    let start_data = serde_json::json!({ "taskId": task_id }).to_string();
    let start = Event::default().event("biz.report.start").data(start_data);
    stream::once(async move { Ok(start) }).chain(stream::unfold(rx, |mut rx| async move {
        let msg = rx.recv().await?;
        let event = stream_msg_to_event(&msg);
        tokio::task::yield_now().await;
        Some((Ok(event), rx))
    }))
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
}
