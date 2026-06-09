/** Turn orchestration swimlane timeline (from gateway API). Author: kejiqing */

export interface TimelineSegment {
  id: string;
  label: string;
  startMs: number;
  endMs: number;
  durationMs: number;
  status: string;
  detail?: string;
}

export interface TimelineLane {
  id: string;
  label: string;
  parallel: boolean;
  segments: TimelineSegment[];
}

export interface PhaseSummary {
  phase: string;
  durationMs: number;
  detail?: string;
}

export interface SolveTurnTimeline {
  originMs: number;
  endMs: number;
  totalMs: number;
  lanes: TimelineLane[];
  phases?: PhaseSummary[];
}

export interface TurnTimelineResponse {
  sessionId: string;
  turnId: string;
  projId: number;
  taskCreatedAtMs?: number;
  taskFinishedAtMs?: number;
  timeline?: SolveTurnTimeline | null;
}
