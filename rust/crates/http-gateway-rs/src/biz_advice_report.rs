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

/// 默认润色说明（skill 未部署时的回退，与历史网关内联 prompt 一致）。
pub fn default_gpos_boss_report_writer_skill_md() -> &'static str {
    r"---
name: GPOS_BOSS_REPORT_WRITER
description: Boss 报表分析输出清洗与润色（去除中间过程，产出最终业务报告；最终报告语言须与用户原始提问语言一致）
---

你是高级商业分析顾问。以下将提供包含中间过程、思考草稿与噪声的原始输出。
请仅输出「最终的干净报告」，要求：
1) 不得输出任何中间过程、思考轨迹或工具调用痕迹。
2) 结构清晰，用简洁自然的语言表达；标题、列表、正文须与用户原始提问使用同一种语言，避免混用其他语言。
3) 保留重要结论、依据与可执行建议。
4) 若信息不足，明确标注并列出最少需要的补充数据（使用与用户原始提问相同的语言）。
5) 不得添加原文中不存在的事实。

以用户附带的【原始文本输出】和【原始 JSON 输出】为唯一事实来源；报告书写语言以用户原始提问的语言为准。
"
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

pub fn build_biz_advice_polish_prompt(
    instructions: &str,
    output_text: &str,
    raw_json: &str,
) -> String {
    format!("{instructions}\n\n【原始文本输出】\n{output_text}\n\n【原始 JSON 输出】\n{raw_json}")
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

pub fn raw_json_from_output(output_json: Option<&Value>) -> String {
    output_json.as_ref().map_or_else(
        || "null".to_string(),
        |v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
    )
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
    fn build_prompt_includes_sections() {
        let p = build_biz_advice_polish_prompt("instr", "text", "{}");
        assert!(p.contains("instr"));
        assert!(p.contains("【原始文本输出】"));
        assert!(p.contains("text"));
        assert!(p.contains("【原始 JSON 输出】"));
    }
}
