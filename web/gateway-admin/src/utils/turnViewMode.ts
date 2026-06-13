/** Turn card view mode: terminal turns replay from DB; active turns poll + report SSE. Author: kejiqing */

const TERMINAL_TURN_STATUSES = new Set(["succeeded", "failed", "cancelled"]);

export function isTerminalTurnStatus(status?: string | null): boolean {
  return TERMINAL_TURN_STATUSES.has((status ?? "").trim());
}

/** `live` = poll task + `biz_advice_report?stream=true` (N terminals each subscribe independently). */
export function turnViewModeForStatus(status?: string | null): "live" | "history" {
  return isTerminalTurnStatus(status) ? "history" : "live";
}

export function isHistoryTurnView(
  viewMode: "live" | "history" | undefined,
  status?: string | null
): boolean {
  return viewMode === "history" && isTerminalTurnStatus(status);
}
