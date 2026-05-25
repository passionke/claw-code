//! Build swimlane timeline from orchestration + progress artifacts. Author: kejiqing

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::multi_agent::event_bus::{OrchestrationEvent, ORCHESTRATION_EVENTS_REL};
use crate::multi_agent::timings::MultiAgentTimings;
use crate::task_progress::{read_progress_events, ProgressEvent, REPORT_PROGRESS_EVENT_KIND};

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

fn read_orchestration_events(session_home: &Path) -> Vec<OrchestrationEvent> {
    let path = session_home.join(ORCHESTRATION_EVENTS_REL);
    if !path.is_file() {
        return Vec::new();
    }
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
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

fn query_title(events: &[OrchestrationEvent], todo_id: &str) -> String {
    events
        .iter()
        .find(|e| e.kind == "query_started" && e.todo_id.as_deref() == Some(todo_id))
        .and_then(|e| e.message.clone())
        .unwrap_or_else(|| format!("query {todo_id}"))
}

/// Build timeline lanes from session `.claw` artifacts.
#[must_use]
pub fn build_solve_turn_timeline(session_home: &Path) -> Option<SolveTurnTimeline> {
    let events = read_orchestration_events(session_home);
    if events.is_empty() {
        return build_progress_only_timeline(session_home);
    }

    let origin = events.first().map(|e| e.ts_ms)?;
    let mut end = events.last().map(|e| e.ts_ms).unwrap_or(origin);

    let mut lanes = Vec::new();

    if let (Some(s0), Some(s1)) = (
        ts(&events, "session_started"),
        ts(&events, "preflight_done"),
    ) {
        lanes.push(lane(
            "preflight",
            "Preflight · SQLBot",
            false,
            vec![seg("preflight", "mcp_start + schema", s0, s1, "ok", None)],
        ));
    }

    let planner_start = ts(&events, "preflight_done").or(ts(&events, "session_started"))?;
    if let Some(plan_at) = ts(&events, "plan_ready") {
        let title = events
            .iter()
            .find(|e| e.kind == "plan_ready")
            .and_then(|e| e.message.clone())
            .unwrap_or_else(|| String::from("分析大纲"));
        lanes.push(lane(
            "planner",
            "Planner · LLM",
            false,
            vec![seg("planner", title, planner_start, plan_at, "ok", None)],
        ));
    }

    let mut query_starts: BTreeMap<String, i64> = BTreeMap::new();
    let mut query_segments = Vec::new();
    for ev in &events {
        match ev.kind.as_str() {
            "query_started" => {
                if let Some(id) = &ev.todo_id {
                    query_starts.entry(id.clone()).or_insert(ev.ts_ms);
                }
            }
            "query_done" => {
                if let Some(id) = &ev.todo_id {
                    let start = query_starts.get(id).copied().unwrap_or_else(|| {
                        ev.duration_ms
                            .map(|d| ev.ts_ms.saturating_sub(d))
                            .unwrap_or(ev.ts_ms)
                    });
                    query_segments.push(seg(
                        id.clone(),
                        query_title(&events, id),
                        start,
                        ev.ts_ms,
                        "ok",
                        ev.duration_ms.map(|d| format!("{d}ms")),
                    ));
                }
            }
            "query_failed" => {
                if let Some(id) = &ev.todo_id {
                    let start = query_starts
                        .get(id)
                        .copied()
                        .unwrap_or(ev.ts_ms.saturating_sub(1));
                    query_segments.push(seg(
                        id.clone(),
                        query_title(&events, id),
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
    query_segments.sort_by_key(|s| s.start_ms);
    if !query_segments.is_empty() {
        end = end.max(query_segments.iter().map(|s| s.end_ms).max().unwrap_or(end));
        lanes.push(lane(
            "query_fanout",
            "SQLBot 问数（并行）",
            true,
            query_segments,
        ));
    }

    if let (Some(w0), Some(w1)) = (ts(&events, "writer_started"), ts(&events, "writer_done")) {
        lanes.push(lane(
            "writer",
            "Report Writer · LLM",
            false,
            vec![seg("writer", "撰写分析报告", w0, w1, "ok", None)],
        ));
        end = end.max(w1);
    }

    if let Ok(progress) = read_progress_events(session_home, 500) {
        let report_events: Vec<&ProgressEvent> = progress
            .iter()
            .filter(|e| e.kind == REPORT_PROGRESS_EVENT_KIND)
            .collect();
        if !report_events.is_empty() {
            let mut prog_segments = Vec::new();
            for (i, ev) in report_events.iter().enumerate() {
                if ev.ts_ms < origin {
                    continue;
                }
                let next_end = report_events.get(i + 1).map(|n| n.ts_ms).unwrap_or(end);
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
}
