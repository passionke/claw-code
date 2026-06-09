/** solve_async playground types. Author: kejiqing */

export interface ProgressEvent {
  kind?: string;
  message?: string;
}

/** Multi-agent plan todo from `task-progress.json`. Author: kejiqing */
export interface TaskProgressTodo {
  id: string;
  title: string;
  status: string;
}

/** `POST /v1/sessions/{sessionId}/turns/{turnId}/cancel`. Author: kejiqing */
export interface TurnCancelResponse {
  sessionId: string;
  turnId: string;
  projId: number;
  status: string;
  cancelApplied: boolean;
  error?: unknown;
}

export interface SolveTask {
  status?: string;
  hasReport?: boolean;
  /** Wall-clock start from `GET /v1/tasks`. Author: kejiqing */
  createdAtMs?: number;
  /** Wall-clock end when terminal. Author: kejiqing */
  finishedAtMs?: number | null;
  /** Report time (ms); set when `hasReport` is true (`running` / `succeeded`). */
  reportTime?: number;
  currentTaskDesc?: string;
  /** Multi-agent analysis framework title. Author: kejiqing */
  planTitle?: string;
  /** Multi-agent todo checklist with status. Author: kejiqing */
  todos?: TaskProgressTodo[];
  progressHistory?: ProgressEvent[];
  result?: { outputText?: string };
  error?: unknown;
  /** Resolved pool from `gateway_turns.pool_id`. Author: kejiqing */
  poolId?: string | null;
  /** Worker container after pool exec. Author: kejiqing */
  workerName?: string | null;
}

export interface SolveAsyncResponse {
  taskId: string;
  sessionId: string;
  turnId: string;
  status?: string;
  poolId?: string | null;
  workerName?: string | null;
}

export interface ChatBubble {
  id: string;
  kind: "user" | "sys";
  tag?: string;
  text: string;
  variant?: "warn" | "err";
}

export interface GatewaySessionSummary {
  sessionId: string;
  createdAtMs: number;
  updatedAtMs: number;
  turnCount: number;
  previewPrompt?: string | null;
  clientOrigin?: string | null;
  /** 任一轮有点踩。Author: kejiqing */
  hasBadFeedback?: boolean;
  /** 任一轮有点赞。Author: kejiqing */
  hasGoodFeedback?: boolean;
}

export interface ListProjectSessionsResponse {
  projId: number;
  sessions: GatewaySessionSummary[];
  hasMore?: boolean;
}

export interface GatewayTurnSummary {
  turnId: string;
  userPrompt?: string | null;
  status: string;
  createdAtMs: number;
  finishedAtMs?: number | null;
  hasReport: boolean;
  /** 已解析的 `output_json.message` / JSON 形 `report_message`，历史回放秒出。Author: kejiqing */
  reportBody?: string | null;
  /** failed 时 `output_json.detail`（solve 真因）。Author: kejiqing */
  failureDetail?: string | null;
  /** 发起方标记（`gateway-admin` 等）。Author: kejiqing */
  clientOrigin?: string | null;
  /** `good` / `bad`，来自 gateway_feedback。Author: kejiqing */
  feedback?: TurnFeedbackValue | null;
  /** Enqueue snapshot from `entry_params_json`. Author: kejiqing */
  extraSession?: Record<string, unknown> | null;
  poolId?: string | null;
  workerName?: string | null;
}

export interface ListSessionTurnsResponse {
  sessionId: string;
  projId: number;
  turns: GatewayTurnSummary[];
}

/** `POST /v1/agent/feedback` · `GET /v1/agent/feedback`. Author: kejiqing */
export type TurnFeedbackValue = "good" | "bad";

export interface AgentFeedbackPostResponse {
  sessionId: string;
  projId: number;
  turnId: string;
  feedback: TurnFeedbackValue;
  updatedAtMs: number;
}

export interface AgentFeedbackGetResponse {
  sessionId: string;
  projId: number;
  items: Record<string, TurnFeedbackValue>;
}

/** `GET /v1/biz_advice_report?stream=false` per turn. Author: kejiqing */
export interface BizAdviceReportResponse {
  reportText?: string;
  sourceStatus?: string;
}
