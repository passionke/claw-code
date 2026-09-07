/** Plan confirm UX helpers (button + chat 「确认」→ same API). Author: kejiqing */

/** `GET /v1/sessions/{id}/plans` row. Author: kejiqing */
export interface SessionPlanRow {
  planId: string;
  sessionId: string;
  projId: number;
  title?: string;
  bodyMarkdown?: string;
  status: string;
  planTurnId?: string | null;
  executeTurnId?: string | null;
  sealedAtMs?: number | null;
  createdAtMs?: number;
  updatedAtMs?: number;
  createdByPrompt?: string | null;
}

export interface ListSessionPlansResponse {
  sessionId: string;
  projId: number;
  plans: SessionPlanRow[];
}

/** Map DB plan status → SolveTask.planPhase for the plan-turn card. Author: kejiqing */
export function planPhaseFromPlanStatus(status: string): string {
  switch (status) {
    case "awaiting_confirm":
      return "awaiting_confirm";
    case "sealed":
      return "confirmed";
    case "superseded":
      return "superseded";
    default:
      return status;
  }
}

/**
 * True when the composer text is a bare confirm (not a new request).
 * Keep exact/short only so "确认品牌改成 X" still goes to normal solve.
 * Author: kejiqing
 */
export function isPlanConfirmUserPrompt(raw: string): boolean {
  const t = raw
    .trim()
    .toLowerCase()
    .replace(/[!！。.~～\s]+$/g, "")
    .trim();
  if (!t || t.length > 24) return false;
  const exact = new Set([
    "确认",
    "确认执行",
    "开始",
    "开始吧",
    "开始执行",
    "同意",
    "可以",
    "好",
    "好的",
    "执行",
    "go",
    "ok",
    "okay",
    "yes",
    "y",
    "confirm",
    "lgtm",
  ]);
  return exact.has(t);
}
