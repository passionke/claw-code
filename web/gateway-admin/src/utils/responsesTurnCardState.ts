/**
 * Pure state helpers for playground POST /v1/responses live turn cards.
 * Locks the same-session next-turn regression: queued + previous-turn duration.
 * Author: kejiqing
 */

import { isTerminalTurnStatus } from "./turnViewMode";

/** Map responses SSE stream state → card `initialStatus`. Author: kejiqing */
export function deriveResponsesCardStatus(state: {
  error?: string;
  completed: boolean;
}): "running" | "succeeded" | "failed" {
  if (state.error) return "failed";
  if (state.completed) return "succeeded";
  return "running";
}

/**
 * `GET /v1/tasks/{sessionId}` may return the previous turn before this one is ready.
 * Only adopt poll results for the card's own turnId. Author: kejiqing
 */
export function shouldAdoptSessionTaskForResponsesCard(
  cardTurnId: string,
  polledTurnId?: string | null
): boolean {
  if (!cardTurnId.trim()) return false;
  if (polledTurnId && polledTurnId !== cardTurnId) return false;
  return true;
}

/** Drop wall-clock fields stolen from another turn / previous terminal poll. Author: kejiqing */
export function shouldDropStaleResponsesTaskTimes(opts: {
  status: string;
  cardTurnId: string;
  taskTurnId?: string;
  finishedAtMs?: number | null;
}): boolean {
  const turnMismatch = Boolean(
    opts.cardTurnId && opts.taskTurnId && opts.taskTurnId !== opts.cardTurnId
  );
  const staleFinishedWhileLive =
    !isTerminalTurnStatus(opts.status) && opts.finishedAtMs != null;
  return turnMismatch || staleFinishedWhileLive;
}

/** Prefer poll times only when they belong to this turn; else fall back to card props. Author: kejiqing */
export function resolveResponsesCardTimestamps(opts: {
  responsesLive: boolean;
  cardTurnId: string;
  taskTurnId?: string;
  taskCreatedAtMs?: number;
  taskFinishedAtMs?: number | null;
  propCreatedAtMs?: number;
  propFinishedAtMs?: number | null;
}): { createdAtMs?: number; finishedAtMs?: number | null } {
  const timesBelongToThisTurn =
    !opts.responsesLive ||
    !opts.cardTurnId ||
    !opts.taskTurnId ||
    opts.taskTurnId === opts.cardTurnId;
  return {
    createdAtMs:
      (timesBelongToThisTurn ? opts.taskCreatedAtMs : undefined) ??
      opts.propCreatedAtMs,
    finishedAtMs:
      (timesBelongToThisTurn ? opts.taskFinishedAtMs : undefined) ??
      opts.propFinishedAtMs,
  };
}

/** Duration badge: only for terminal status; never while stuck "queued" with old finishedAt. Author: kejiqing */
export function resolveTurnCardWallMs(opts: {
  status: string;
  createdAtMs?: number;
  finishedAtMs?: number | null;
}): number | null {
  if (!isTerminalTurnStatus(opts.status)) return null;
  if (
    opts.createdAtMs == null ||
    opts.finishedAtMs == null ||
    opts.finishedAtMs < opts.createdAtMs
  ) {
    return null;
  }
  return opts.finishedAtMs - opts.createdAtMs;
}

/** finishedAtMs for live card patches from SSE. Author: kejiqing */
export function nextResponsesFinishedAtMs(
  status: "running" | "succeeded" | "failed",
  prevFinishedAtMs: number | null | undefined,
  nowMs: number
): number | undefined {
  if (status === "running") return undefined;
  return prevFinishedAtMs ?? nowMs;
}
