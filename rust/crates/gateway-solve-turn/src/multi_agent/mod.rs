//! Multi-agent phased gateway solve. Author: kejiqing

mod event_bus;
mod narrator_lane;
mod phase_turn;
mod phases;
mod plan;
mod planner_turn;
mod progress_sync;
mod query_fanout;
mod timeline;
mod timings;
mod writer_turn;

pub mod orchestrator;

pub use event_bus::{EventBus, ORCHESTRATION_EVENTS_REL};
pub use orchestrator::run_multi_agent_solve_turn;
pub use plan::{AnalysisPlan, AnalysisPlanTodo};
pub use timeline::{
    build_solve_turn_timeline, build_solve_turn_timeline_for_turn,
    build_solve_turn_timeline_from_timing_json, SolveTurnTimeline, TimelineLane, TimelineSegment,
    TurnTimelineWindow,
};
pub use timings::now_ms;
