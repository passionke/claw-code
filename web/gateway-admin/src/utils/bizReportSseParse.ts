/** Pure SSE payload parsers for live biz report stream. Author: kejiqing */

import { extractSolveReportMessage } from "./solveReportBody";

export function parseBizReportSseJson(raw: string): Record<string, unknown> | null {
  try {
    return JSON.parse(raw) as Record<string, unknown>;
  } catch {
    return null;
  }
}

export function bizReportNumField(
  data: Record<string, unknown> | null,
  key: string
): number | undefined {
  const v = data?.[key];
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

/** Full report from `biz.report.done` (pool live or gateway snapshot). Author: kejiqing */
export function reportTextFromBizReportDone(
  data: Record<string, unknown> | null
): string {
  if (!data) return "";
  const direct = data.reportText;
  if (typeof direct === "string" && direct.trim()) {
    return extractSolveReportMessage(direct);
  }
  const rj = data.reportJson ?? data.report_json;
  if (rj && typeof rj === "object" && rj !== null) {
    const msg = (rj as Record<string, unknown>).message;
    if (typeof msg === "string" && msg.trim()) {
      return extractSolveReportMessage(msg);
    }
  }
  return "";
}

export function bizReportDeltaText(data: Record<string, unknown> | null): string {
  const chunk = data?.text;
  return chunk != null ? String(chunk) : "";
}
