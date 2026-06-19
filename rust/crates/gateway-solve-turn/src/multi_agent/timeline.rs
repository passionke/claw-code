//! Build swimlane timeline from orchestration + progress artifacts. Author: kejiqing

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::multi_agent::event_bus::{OrchestrationEvent, ORCHESTRATION_EVENTS_REL};
use crate::multi_agent::timings::MultiAgentTimings;
use crate::solve_timing::{
    filter_solve_timing_events_for_window, read_solve_timing_events, SolveTimingEvent,
};
use crate::task_progress::{read_progress_events, ProgressEvent, REPORT_PROGRESS_EVENT_KIND};

/// Wall-clock window for one gateway turn (`gateway_turns.created_at_ms` .. `finished_at_ms`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnTimelineWindow {
    pub from_ms: i64,
    pub to_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSegment {
    pub id: String,
    pub label: String,
    #[serde(rename = "startMs")]
    pub start_ms: i64,
    #[serde(rename = "endMs")]
    pub end_ms: i64,
    #[serde(rename = "durationMs")]
    pub duration_ms: i64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineLane {
    pub id: String,
    pub label: String,
    pub parallel: bool,
    pub segments: Vec<TimelineSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SolveTurnTimeline {
    #[serde(rename = "originMs")]
    pub origin_ms: i64,
    #[serde(rename = "endMs")]
    pub end_ms: i64,
    #[serde(rename = "totalMs")]
    pub total_ms: i64,
    pub lanes: Vec<TimelineLane>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<PhaseSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PhaseSummary {
    pub phase: String,
    #[serde(rename = "durationMs")]
    pub duration_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Parse NDJSON orchestration lines (pool readback / DB store). Author: kejiqing
#[must_use]
pub fn parse_orchestration_events_ndjson(raw: &str) -> Vec<OrchestrationEvent> {
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
    out
}

fn read_orchestration_events(session_home: &Path) -> Vec<OrchestrationEvent> {
    let path = session_home.join(ORCHESTRATION_EVENTS_REL);
    if !path.is_file() {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_orchestration_events_ndjson(&raw)
}

fn seg(
    id: impl Into<String>,
    label: impl Into<String>,
    start_ms: i64,
    end_ms: i64,
    status: impl Into<String>,
    detail: Option<String>,
) -> TimelineSegment {
    let start_ms = start_ms.max(0);
    let end_ms = end_ms.max(start_ms);
    TimelineSegment {
        id: id.into(),
        label: label.into(),
        start_ms,
        end_ms,
        duration_ms: end_ms.saturating_sub(start_ms),
        status: status.into(),
        detail,
    }
}

fn lane(
    id: impl Into<String>,
    label: impl Into<String>,
    parallel: bool,
    segments: Vec<TimelineSegment>,
) -> TimelineLane {
    TimelineLane {
        id: id.into(),
        label: label.into(),
        parallel,
        segments,
    }
}

fn ts(events: &[OrchestrationEvent], kind: &str) -> Option<i64> {
    events.iter().find(|e| e.kind == kind).map(|e| e.ts_ms)
}

fn fanout_item_title(
    events: &[OrchestrationEvent],
    started_kind: &str,
    item_id: &str,
    fallback_prefix: &str,
) -> String {
    events
        .iter()
        .find(|e| e.kind == started_kind && e.todo_id.as_deref() == Some(item_id))
        .and_then(|e| e.message.clone())
        .unwrap_or_else(|| format!("{fallback_prefix} {item_id}"))
}

fn build_parallel_fanout_segments(
    events: &[OrchestrationEvent],
    started_kind: &str,
    done_kind: &str,
    failed_kind: &str,
    title_prefix: &str,
) -> Vec<TimelineSegment> {
    let mut starts: BTreeMap<String, i64> = BTreeMap::new();
    let mut segments = Vec::new();
    for ev in events {
        match ev.kind.as_str() {
            k if k == started_kind => {
                if let Some(id) = &ev.todo_id {
                    starts.entry(id.clone()).or_insert(ev.ts_ms);
                }
            }
            k if k == done_kind => {
                if let Some(id) = &ev.todo_id {
                    let start = starts.get(id).copied().unwrap_or_else(|| {
                        ev.duration_ms
                            .map(|d| ev.ts_ms.saturating_sub(d))
                            .unwrap_or(ev.ts_ms)
                    });
                    segments.push(seg(
                        id.clone(),
                        fanout_item_title(events, started_kind, id, title_prefix),
                        start,
                        ev.ts_ms,
                        "ok",
                        ev.duration_ms.map(|d| format!("{d}ms")),
                    ));
                }
            }
            k if k == failed_kind => {
                if let Some(id) = &ev.todo_id {
                    let start = starts
                        .get(id)
                        .copied()
                        .unwrap_or(ev.ts_ms.saturating_sub(1));
                    segments.push(seg(
                        id.clone(),
                        fanout_item_title(events, started_kind, id, title_prefix),
                        start,
                        ev.ts_ms,
                        "failed",
                        ev.error.clone(),
                    ));
                }
            }
            _ => {}
        }
    }
    segments.sort_by_key(|s| s.start_ms);
    segments
}

fn filter_orchestration_events(
    events: &[OrchestrationEvent],
    window: TurnTimelineWindow,
) -> Vec<OrchestrationEvent> {
    events
        .iter()
        .filter(|e| e.ts_ms >= window.from_ms && e.ts_ms <= window.to_ms)
        .cloned()
        .collect()
}

fn filter_progress_events(
    events: &[ProgressEvent],
    window: TurnTimelineWindow,
) -> Vec<ProgressEvent> {
    events
        .iter()
        .filter(|e| e.ts_ms >= window.from_ms && e.ts_ms <= window.to_ms)
        .cloned()
        .collect()
}

fn short_tool_label(name: &str) -> String {
    let n = name.trim();
    if n.is_empty() {
        return String::from("tool");
    }
    if n.len() > 48 {
        format!("{}…", n.chars().take(45).collect::<String>())
    } else {
        n.to_string()
    }
}

fn bootstrap_label(kind: &str) -> &'static str {
    match kind {
        "bootstrap_solve_pool_start" => "网关 · 写 task / 解析 LLM",
        "bootstrap_pool_acquired" => "Pool · 租 slot + tar 灌入",
        "bootstrap_exec_started" => "Pool · docker exec 启动",
        "bootstrap_worker_entered" => "Worker · gateway-solve-once",
        "bootstrap_mcp_ready" => "Worker · MCP discover",
        "session_started" => "Orchestration · 开始",
        _ => "启动",
    }
}

fn build_bootstrap_lane(
    origin_ms: i64,
    timing: &[SolveTimingEvent],
    orch: &[OrchestrationEvent],
) -> Option<TimelineLane> {
    let mut points: Vec<(i64, String, String)> = Vec::new();
    for ev in timing {
        if ev.kind.starts_with("bootstrap_") {
            points.push((
                ev.ts_ms,
                ev.kind.clone(),
                bootstrap_label(&ev.kind).to_string(),
            ));
        }
    }
    if let Some(ts) = ts(orch, "session_started") {
        points.push((
            ts,
            "session_started".to_string(),
            bootstrap_label("session_started").to_string(),
        ));
    }
    if points.is_empty() {
        return None;
    }
    points.sort_by_key(|(ts, _, _)| *ts);
    points.dedup_by_key(|(ts, kind, _)| (*ts, kind.clone()));

    let mut segments = Vec::new();
    let mut prev = origin_ms;
    for (i, (ts, kind, label)) in points.iter().enumerate() {
        if *ts <= prev {
            continue;
        }
        segments.push(seg(
            format!("bootstrap-{i}-{kind}"),
            label.clone(),
            prev,
            *ts,
            "info",
            Some(format!("{}ms", ts.saturating_sub(prev))),
        ));
        prev = *ts;
    }
    if segments.is_empty() {
        return None;
    }
    Some(lane("bootstrap", "启动 · 排队到开始工作", false, segments))
}

fn build_timing_lanes(events: &[SolveTimingEvent]) -> Vec<TimelineLane> {
    let mut llm_segments = Vec::new();
    let mut tool_segments = Vec::new();
    let mut tool_starts: BTreeMap<String, (String, i64)> = BTreeMap::new();

    for ev in events {
        match ev.kind.as_str() {
            "llm_stream_finished" => {
                let dur = ev.duration_ms.unwrap_or(0).max(0);
                let end = ev.ts_ms;
                let start = end.saturating_sub(dur);
                let iter = ev.iteration.unwrap_or(0);
                llm_segments.push(seg(
                    format!("llm-{iter}"),
                    format!("LLM · iter {iter}"),
                    start,
                    end,
                    "ok",
                    Some(format!("{dur}ms")),
                ));
            }
            "llm_stream_failed" => {
                let dur = ev.duration_ms.unwrap_or(0).max(0);
                let end = ev.ts_ms;
                let start = end.saturating_sub(dur);
                let iter = ev.iteration.unwrap_or(0);
                llm_segments.push(seg(
                    format!("llm-fail-{iter}"),
                    format!("LLM · iter {iter}"),
                    start,
                    end,
                    "failed",
                    ev.error.clone(),
                ));
            }
            "tool_execution_started" => {
                if let Some(id) = &ev.tool_use_id {
                    let name = ev.tool_name.as_deref().unwrap_or("tool");
                    tool_starts.insert(id.clone(), (name.to_string(), ev.ts_ms));
                }
            }
            "tool_execution_finished" => {
                let id = ev
                    .tool_use_id
                    .clone()
                    .unwrap_or_else(|| format!("tool-finish-{}", tool_segments.len()));
                let name = ev.tool_name.as_deref().unwrap_or("tool");
                let end = ev.ts_ms;
                let start = tool_starts
                    .get(&id)
                    .map(|(_, s)| *s)
                    .unwrap_or_else(|| end.saturating_sub(ev.duration_ms.unwrap_or(0).max(0)));
                let status = if ev.is_error == Some(true) {
                    "failed"
                } else {
                    "ok"
                };
                let detail = ev.duration_ms.map(|d| format!("{d}ms"));
                tool_segments.push(seg(id, short_tool_label(name), start, end, status, detail));
            }
            _ => {}
        }
    }

    llm_segments.sort_by_key(|s| s.start_ms);
    tool_segments.sort_by_key(|s| s.start_ms);

    let mut lanes = Vec::new();
    if !llm_segments.is_empty() {
        lanes.push(lane("llm", "LLM 推理", false, llm_segments));
    }
    if !tool_segments.is_empty() {
        lanes.push(lane("tools", "Tool 执行", false, tool_segments));
    }
    lanes
}

fn max_segment_end(lanes: &[TimelineLane], fallback: i64) -> i64 {
    lanes
        .iter()
        .flat_map(|l| l.segments.iter())
        .map(|s| s.end_ms)
        .max()
        .unwrap_or(fallback)
}

fn build_orchestration_lanes(
    events: &[OrchestrationEvent],
    progress_events: &[ProgressEvent],
    origin: i64,
    end: &mut i64,
) -> Vec<TimelineLane> {
    let mut lanes = Vec::new();

    if let (Some(s0), Some(s1)) = (ts(events, "session_started"), ts(events, "preflight_done")) {
        lanes.push(lane(
            "preflight",
            "Preflight · SQLBot",
            false,
            vec![seg("preflight", "mcp_start + schema", s0, s1, "ok", None)],
        ));
    }

    let planner_start = ts(events, "preflight_done").or_else(|| ts(events, "session_started"));
    if let (Some(ps), Some(plan_at)) = (planner_start, ts(events, "plan_ready")) {
        let title = events
            .iter()
            .find(|e| e.kind == "plan_ready")
            .and_then(|e| e.message.clone())
            .unwrap_or_else(|| String::from("分析大纲"));
        lanes.push(lane(
            "planner",
            "Planner · LLM",
            false,
            vec![seg("planner", title, ps, plan_at, "ok", None)],
        ));
    }

    let query_segments = build_parallel_fanout_segments(
        events,
        "query_started",
        "query_done",
        "query_failed",
        "query",
    );
    if !query_segments.is_empty() {
        *end = (*end).max(
            query_segments
                .iter()
                .map(|s| s.end_ms)
                .max()
                .unwrap_or(*end),
        );
        lanes.push(lane(
            "query_fanout",
            "SQLBot 问数（并行）",
            true,
            query_segments,
        ));
    }

    let agent_segments = build_parallel_fanout_segments(
        events,
        "agent_started",
        "agent_done",
        "agent_failed",
        "agent",
    );
    if !agent_segments.is_empty() {
        *end = (*end).max(
            agent_segments
                .iter()
                .map(|s| s.end_ms)
                .max()
                .unwrap_or(*end),
        );
        lanes.push(lane(
            "agent_fanout",
            "子代理 Agent（并行）",
            true,
            agent_segments,
        ));
    }

    if let (Some(w0), Some(w1)) = (ts(events, "writer_started"), ts(events, "writer_done")) {
        lanes.push(lane(
            "writer",
            "Report Writer · LLM",
            false,
            vec![seg("writer", "撰写分析报告", w0, w1, "ok", None)],
        ));
        *end = (*end).max(w1);
    }

    {
        let report_events: Vec<&ProgressEvent> = progress_events
            .iter()
            .filter(|e| {
                e.kind == REPORT_PROGRESS_EVENT_KIND && e.ts_ms >= origin && e.ts_ms <= *end
            })
            .collect();
        if !report_events.is_empty() {
            let mut prog_segments = Vec::new();
            for (i, ev) in report_events.iter().enumerate() {
                let next_end = report_events.get(i + 1).map(|n| n.ts_ms).unwrap_or(*end);
                let label = ev.message.trim();
                if label.is_empty() {
                    continue;
                }
                prog_segments.push(seg(
                    format!("progress-{i}"),
                    label,
                    ev.ts_ms,
                    next_end.max(ev.ts_ms + 1),
                    "info",
                    None,
                ));
            }
            if !prog_segments.is_empty() {
                lanes.push(lane(
                    "progress",
                    "进度播报（用户可见）",
                    false,
                    prog_segments,
                ));
            }
        }
    }

    lanes
}

fn build_solve_turn_timeline_core(
    orch: &[OrchestrationEvent],
    timing: &[SolveTimingEvent],
    progress: &[ProgressEvent],
    created_at_ms: i64,
    finished_at_ms: Option<i64>,
) -> SolveTurnTimeline {
    let from_ms = created_at_ms;
    let mut to_ms = finished_at_ms.unwrap_or(created_at_ms.saturating_add(1));
    if to_ms < from_ms {
        to_ms = from_ms;
    }
    let window = TurnTimelineWindow { from_ms, to_ms };
    let progress = filter_progress_events(progress, window);

    let mut lanes = build_orchestration_lanes(orch, &progress, from_ms, &mut to_ms);
    if let Some(bootstrap) = build_bootstrap_lane(from_ms, timing, orch) {
        lanes.insert(0, bootstrap);
    }
    lanes.extend(build_timing_lanes(timing));

    if lanes.is_empty() {
        if !progress.is_empty() {
            let origin = from_ms;
            let end = progress
                .last()
                .map(|e| e.ts_ms.max(window.to_ms))
                .unwrap_or(window.to_ms);
            let segments: Vec<TimelineSegment> = progress
                .iter()
                .enumerate()
                .map(|(i, ev)| {
                    let next = progress.get(i + 1).map(|n| n.ts_ms).unwrap_or(end);
                    seg(
                        format!("p-{i}"),
                        ev.message.clone(),
                        ev.ts_ms,
                        next.max(ev.ts_ms + 1),
                        ev.kind.clone(),
                        None,
                    )
                })
                .collect();
            return SolveTurnTimeline {
                origin_ms: origin,
                end_ms: end,
                total_ms: end.saturating_sub(origin),
                lanes: vec![lane("progress", "执行进度", false, segments)],
                phases: Vec::new(),
            };
        }
        return SolveTurnTimeline {
            origin_ms: from_ms,
            end_ms: to_ms,
            total_ms: to_ms.saturating_sub(from_ms),
            lanes: Vec::new(),
            phases: Vec::new(),
        };
    }

    to_ms = to_ms.max(max_segment_end(&lanes, from_ms));

    SolveTurnTimeline {
        origin_ms: from_ms,
        end_ms: to_ms,
        total_ms: to_ms.saturating_sub(from_ms),
        lanes,
        phases: Vec::new(),
    }
}

/// Build timeline from `gateway_turns.solve_timing_jsonb` (pool v1). Author: kejiqing
#[must_use]
pub fn build_solve_turn_timeline_from_timing_json(
    store: &serde_json::Value,
    created_at_ms: i64,
    finished_at_ms: Option<i64>,
) -> Option<SolveTurnTimeline> {
    let timing: Vec<SolveTimingEvent> = store
        .get("solveTimingEvents")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let orch: Vec<OrchestrationEvent> = store
        .get("orchestrationEvents")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let progress: Vec<ProgressEvent> = store
        .get("progressEvents")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let from_ms = created_at_ms;
    let mut to_ms = finished_at_ms.unwrap_or(created_at_ms.saturating_add(1));
    if to_ms < from_ms {
        to_ms = from_ms;
    }
    let window = TurnTimelineWindow { from_ms, to_ms };
    let orch = filter_orchestration_events(&orch, window);
    let timing = filter_solve_timing_events_for_window(&timing, from_ms, to_ms);
    Some(build_solve_turn_timeline_core(
        &orch,
        &timing,
        &progress,
        created_at_ms,
        finished_at_ms,
    ))
}

/// Build timeline for one turn using DB wall-clock bounds and `.claw` artifacts in that window.
#[must_use]
pub fn build_solve_turn_timeline_for_turn(
    session_home: &Path,
    created_at_ms: i64,
    finished_at_ms: Option<i64>,
) -> Option<SolveTurnTimeline> {
    let from_ms = created_at_ms;
    let mut to_ms = finished_at_ms.unwrap_or(created_at_ms.saturating_add(1));
    if to_ms < from_ms {
        to_ms = from_ms;
    }
    let window = TurnTimelineWindow { from_ms, to_ms };

    let all_orch = read_orchestration_events(session_home);
    let orch = filter_orchestration_events(&all_orch, window);

    let timing_all = read_solve_timing_events(session_home, 500).unwrap_or_default();
    let timing = filter_solve_timing_events_for_window(&timing_all, from_ms, to_ms);
    let progress = read_progress_events(session_home, 500).unwrap_or_default();

    Some(build_solve_turn_timeline_core(
        &orch,
        &timing,
        &progress,
        created_at_ms,
        finished_at_ms,
    ))
}

/// Build timeline lanes from session `.claw` artifacts (legacy: whole session, no turn window).
#[must_use]
pub fn build_solve_turn_timeline(session_home: &Path) -> Option<SolveTurnTimeline> {
    let events = read_orchestration_events(session_home);
    if events.is_empty() {
        return build_progress_only_timeline(session_home);
    }

    let origin = events.first().map(|e| e.ts_ms)?;
    let mut end = events.last().map(|e| e.ts_ms).unwrap_or(origin);

    let progress = read_progress_events(session_home, 500).unwrap_or_default();
    let mut lanes = build_orchestration_lanes(&events, &progress, origin, &mut end);

    let timing_all = read_solve_timing_events(session_home, 500).unwrap_or_default();
    lanes.extend(build_timing_lanes(&timing_all));
    end = end.max(max_segment_end(&lanes, origin));

    let timings = MultiAgentTimings::load(session_home);
    let phases: Vec<PhaseSummary> = timings
        .phases
        .iter()
        .map(|p| PhaseSummary {
            phase: p.phase.clone(),
            duration_ms: p.ended_at_ms.saturating_sub(p.started_at_ms),
            detail: p.detail.clone(),
        })
        .collect();

    Some(SolveTurnTimeline {
        origin_ms: origin,
        end_ms: end,
        total_ms: end.saturating_sub(origin),
        lanes,
        phases,
    })
}

fn build_progress_only_timeline(session_home: &Path) -> Option<SolveTurnTimeline> {
    let progress = read_progress_events(session_home, 500).ok()?;
    if progress.is_empty() {
        return None;
    }
    let origin = progress.first()?.ts_ms;
    let end = progress.last()?.ts_ms;
    let segments: Vec<TimelineSegment> = progress
        .iter()
        .enumerate()
        .map(|(i, ev)| {
            let next = progress.get(i + 1).map(|n| n.ts_ms).unwrap_or(end);
            seg(
                format!("p-{i}"),
                ev.message.clone(),
                ev.ts_ms,
                next.max(ev.ts_ms + 1),
                ev.kind.clone(),
                None,
            )
        })
        .collect();
    Some(SolveTurnTimeline {
        origin_ms: origin,
        end_ms: end,
        total_ms: end.saturating_sub(origin),
        lanes: vec![lane("progress", "执行进度", false, segments)],
        phases: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_events(session_home: &Path, lines: &[&str]) {
        std::fs::create_dir_all(session_home.join(".claw")).unwrap();
        let path = session_home.join(ORCHESTRATION_EVENTS_REL);
        let mut f = fs::File::create(path).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
    }

    #[test]
    fn builds_parallel_query_lanes() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_events(
            home,
            &[
                r#"{"kind":"session_started","tsMs":1000}"#,
                r#"{"kind":"preflight_done","tsMs":1500}"#,
                r#"{"kind":"plan_ready","tsMs":5000,"message":"计划"}"#,
                r#"{"kind":"query_started","tsMs":5010,"todoId":"1","message":"Q1"}"#,
                r#"{"kind":"query_started","tsMs":5011,"todoId":"2","message":"Q2"}"#,
                r#"{"kind":"query_done","tsMs":8000,"todoId":"1","durationMs":2990}"#,
                r#"{"kind":"query_done","tsMs":9000,"todoId":"2","durationMs":3989}"#,
                r#"{"kind":"writer_started","tsMs":9010}"#,
                r#"{"kind":"writer_done","tsMs":10000}"#,
            ],
        );
        let t = build_solve_turn_timeline(home).unwrap();
        assert_eq!(t.total_ms, 9000);
        let queries = t.lanes.iter().find(|l| l.id == "query_fanout").unwrap();
        assert!(queries.parallel);
        assert_eq!(queries.segments.len(), 2);
    }

    #[test]
    fn builds_parallel_agent_lanes() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_events(
            home,
            &[
                r#"{"kind":"session_started","tsMs":1000}"#,
                r#"{"kind":"preflight_done","tsMs":1500}"#,
                r#"{"kind":"agent_started","tsMs":2000,"todoId":"a1","message":"营收"}"#,
                r#"{"kind":"agent_started","tsMs":2001,"todoId":"a2","message":"品类"}"#,
                r#"{"kind":"agent_done","tsMs":5000,"todoId":"a1","durationMs":3000}"#,
                r#"{"kind":"agent_done","tsMs":6000,"todoId":"a2","durationMs":3999}"#,
            ],
        );
        let t = build_solve_turn_timeline(home).unwrap();
        let agents = t.lanes.iter().find(|l| l.id == "agent_fanout").unwrap();
        assert!(agents.parallel);
        assert_eq!(agents.segments.len(), 2);
    }

    fn write_timing_events(session_home: &Path, lines: &[&str]) {
        use crate::solve_timing::SOLVE_TIMING_EVENTS_REL;
        std::fs::create_dir_all(session_home.join(".claw")).unwrap();
        let path = session_home.join(SOLVE_TIMING_EVENTS_REL);
        let mut f = fs::File::create(path).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
    }

    #[test]
    fn turn_window_excludes_other_turn_orchestration() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_events(
            home,
            &[
                r#"{"kind":"session_started","tsMs":1000}"#,
                r#"{"kind":"preflight_done","tsMs":1500}"#,
                r#"{"kind":"session_started","tsMs":600_000}"#,
            ],
        );
        let t = build_solve_turn_timeline_for_turn(home, 600_000, Some(638_000)).unwrap();
        assert_eq!(t.origin_ms, 600_000);
        assert_eq!(t.total_ms, 38_000);
        assert!(t.lanes.iter().all(|l| l.id != "preflight"));
    }

    #[test]
    fn turn_window_includes_llm_and_tool_lanes_from_timing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_timing_events(
            home,
            &[
                r#"{"kind":"llm_stream_finished","tsMs":602000,"iteration":1,"durationMs":8000}"#,
                r#"{"kind":"tool_execution_started","tsMs":602100,"toolUseId":"tu1","toolName":"mcp__sqlbot__q"}"#,
                r#"{"kind":"tool_execution_finished","tsMs":610000,"toolUseId":"tu1","toolName":"mcp__sqlbot__q","durationMs":7900,"isError":false}"#,
                r#"{"kind":"llm_stream_finished","tsMs":612000,"iteration":2,"durationMs":2000}"#,
            ],
        );
        let t = build_solve_turn_timeline_for_turn(home, 600_000, Some(615_000)).unwrap();
        assert_eq!(t.origin_ms, 600_000);
        assert!(t.lanes.iter().any(|l| l.id == "llm"));
        let llm = t.lanes.iter().find(|l| l.id == "llm").unwrap();
        assert_eq!(llm.segments.len(), 2);
        let tools = t.lanes.iter().find(|l| l.id == "tools").unwrap();
        assert_eq!(tools.segments.len(), 1);
        assert_eq!(tools.segments[0].duration_ms, 7900);
    }

    #[test]
    fn turn_window_builds_bootstrap_startup_lane() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_events(home, &[r#"{"kind":"session_started","tsMs":20300}"#]);
        write_timing_events(
            home,
            &[
                r#"{"kind":"bootstrap_solve_pool_start","tsMs":10050,"source":"bootstrap"}"#,
                r#"{"kind":"bootstrap_pool_acquired","tsMs":20000,"source":"bootstrap"}"#,
                r#"{"kind":"bootstrap_worker_entered","tsMs":20100,"source":"bootstrap"}"#,
                r#"{"kind":"bootstrap_mcp_ready","tsMs":20200,"source":"bootstrap"}"#,
            ],
        );
        let t = build_solve_turn_timeline_for_turn(home, 10_000, Some(25_000)).unwrap();
        let boot = t
            .lanes
            .iter()
            .find(|l| l.id == "bootstrap")
            .expect("bootstrap lane");
        assert_eq!(boot.segments.len(), 5);
        assert_eq!(boot.segments[0].duration_ms, 50);
        assert_eq!(boot.segments[1].duration_ms, 9950);
        assert_eq!(boot.segments[4].label, "Orchestration · 开始");
    }
}
