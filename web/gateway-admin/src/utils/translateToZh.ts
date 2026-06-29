/** 整通对话译中文：后端编排 + PG 快照，前端只触发与取结果。Author: kejiqing */

import { proxyHttp } from "../api/client";

export type ConversationTranslateStatus = "translating" | "ready" | "error";

export interface TranslatedTurn {
  index: number;
  turnId: string;
  userText: string;
  assistantText: string;
  userTextZh: string;
  assistantTextZh: string;
}

export interface ConversationTranslateSnapshot {
  sourceFingerprint: string;
  turns: TranslatedTurn[];
  markdown: string;
  targetLanguage: string;
  modelId?: string;
  status: ConversationTranslateStatus;
  /** 已完成快照对应的源正文是否已变化（有新轮次完成）。 */
  stale: boolean;
  error?: string;
  updatedAtMs: number;
}

/** 读取当前会话的译文快照（含 status / stale / error）。 */
export async function loadConversationTranslateSnapshot(
  gatewayBase: string,
  sessionId: string,
  projId: number
): Promise<ConversationTranslateSnapshot | null> {
  const res = await proxyHttp<{ snapshot?: ConversationTranslateSnapshot | null }>(
    gatewayBase,
    "GET",
    `/v1/sessions/${encodeURIComponent(sessionId)}/conversation_translate?proj_id=${encodeURIComponent(String(projId))}`
  );
  return res.snapshot ?? null;
}

/** 触发后端重新翻译整通会话（异步）；并发触发会被后端单飞锁拒绝。 */
export async function triggerConversationTranslate(
  gatewayBase: string,
  sessionId: string,
  projId: number
): Promise<{ status: string }> {
  return proxyHttp<{ ok: boolean; status: string }>(
    gatewayBase,
    "POST",
    `/v1/sessions/${encodeURIComponent(sessionId)}/conversation_translate?proj_id=${encodeURIComponent(String(projId))}`
  );
}
