/** 从当前会话线程收集各轮用户/助手正文（复用既有网关 API）。Author: kejiqing */

import { proxyHttp } from "../api/client";
import type { BizAdviceReportResponse, GatewayTurnSummary, SolveTask } from "../types/chat";
import { extractSolveReportMessage } from "./solveReportBody";

export interface ConversationTurnInput {
  turnId: string;
  sessionId: string;
  taskId: string;
  userText: string;
  viewMode?: "live" | "history";
  historicalReport?: string;
  failureDetail?: string;
}

export interface ConversationTurnBlock {
  index: number;
  turnId: string;
  userText: string;
  assistantText: string;
}

async function fetchTurnReport(
  gatewayBase: string,
  sessionId: string,
  turnId: string,
  projId: number
): Promise<string> {
  const q = new URLSearchParams({
    sessionId,
    turnId,
    proj_id: String(projId),
    stream: "false",
  });
  const res = await proxyHttp<BizAdviceReportResponse>(
    gatewayBase,
    "GET",
    `/v1/biz_advice_report?${q.toString()}`
  );
  return extractSolveReportMessage(res.reportText?.trim() ?? "");
}

async function fetchTaskAssistantText(
  gatewayBase: string,
  taskId: string
): Promise<string> {
  const t = await proxyHttp<SolveTask>(gatewayBase, "GET", `/v1/tasks/${encodeURIComponent(taskId)}`);
  if (t.error) return JSON.stringify(t.error, null, 2);
  const fromResult = extractSolveReportMessage(t.result?.outputText?.trim() ?? "");
  if (fromResult) return fromResult;
  if (t.status === "failed") return "（任务失败，无报告正文）";
  if (t.status === "cancelled") return "（任务已取消）";
  if (t.status === "running" || t.status === "queued") {
    return t.currentTaskDesc?.trim() || "（任务进行中，暂无完整报告）";
  }
  return "";
}

async function resolveAssistantText(
  gatewayBase: string,
  projId: number,
  turn: ConversationTurnInput
): Promise<string> {
  const failure = turn.failureDetail?.trim();
  if (failure) return failure;

  const prefilled = extractSolveReportMessage(turn.historicalReport?.trim() ?? "");
  if (prefilled) return prefilled;

  try {
    const report = await fetchTurnReport(gatewayBase, turn.sessionId, turn.turnId, projId);
    if (report) return report;
  } catch {
    /* fall through */
  }

  if (turn.viewMode !== "history") {
    try {
      const live = await fetchTaskAssistantText(gatewayBase, turn.taskId);
      if (live) return live;
    } catch {
      /* fall through */
    }
  }

  return "（该轮次无已持久化的助手内容）";
}

/** 按当前 thread 中的 turn 顺序拉齐助手正文。 */
export async function collectConversationTurns(
  gatewayBase: string,
  projId: number,
  turns: ConversationTurnInput[],
  onProgress?: (done: number, total: number) => void
): Promise<ConversationTurnBlock[]> {
  const total = turns.length;
  const blocks: ConversationTurnBlock[] = [];
  for (let i = 0; i < turns.length; i += 1) {
    const t = turns[i];
    const assistantText = await resolveAssistantText(gatewayBase, projId, t);
    blocks.push({
      index: i + 1,
      turnId: t.turnId,
      userText: t.userText.trim() || "（无用户文案）",
      assistantText,
    });
    onProgress?.(i + 1, total);
  }
  return blocks;
}

/** 从历史会话 API 一次性加载（侧边栏选中会话时 thread 可能尚未渲染完）。 */
export async function loadSessionTurnsForTranslate(
  gatewayBase: string,
  projId: number,
  sessionId: string
): Promise<ConversationTurnInput[]> {
  const res = await proxyHttp<{ turns?: GatewayTurnSummary[] }>(
    gatewayBase,
    "GET",
    `/v1/sessions/${encodeURIComponent(sessionId)}/turns?proj_id=${encodeURIComponent(String(projId))}`
  );
  return (res.turns ?? []).map((t) => ({
    turnId: t.turnId,
    sessionId,
    taskId: sessionId,
    userText: t.userPrompt?.trim() || "（无用户文案）",
    viewMode: "history" as const,
    historicalReport: t.reportBody ? extractSolveReportMessage(t.reportBody) : undefined,
    failureDetail: t.failureDetail?.trim() || undefined,
  }));
}
