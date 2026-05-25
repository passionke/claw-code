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
  dsId: number;
  status: string;
  cancelApplied: boolean;
  error?: unknown;
}

export interface SolveTask {
  status?: string;
  hasReport?: boolean;
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
}

export interface SolveAsyncResponse {
  taskId: string;
  sessionId: string;
  turnId: string;
  status?: string;
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
}

export interface ListProjectSessionsResponse {
  dsId: number;
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
}

export interface ListSessionTurnsResponse {
  sessionId: string;
  dsId: number;
  turns: GatewayTurnSummary[];
}

/** `GET /v1/biz_advice_report?stream=false` per turn. Author: kejiqing */
export interface BizAdviceReportResponse {
  reportText?: string;
  sourceStatus?: string;
}
