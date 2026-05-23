/** solve_async playground types. Author: kejiqing */

export interface ProgressEvent {
  kind?: string;
  message?: string;
}

export interface SolveTask {
  status?: string;
  hasReport?: boolean;
  /** Report time (ms); set when `hasReport` is true (`running` / `succeeded`). */
  reportTime?: number;
  currentTaskDesc?: string;
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
