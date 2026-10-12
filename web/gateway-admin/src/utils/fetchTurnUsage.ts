/** Soft-poll turn usage after stream ends (tap INSERT race). Author: kejiqing */

import { proxyHttp } from "../api/client";
import type { TurnUsageAggregate } from "../types/chat";
import { parseTurnUsageAggregate } from "./tokenUsage";

export interface TurnUsageApiResponse {
  sessionId?: string;
  turnId?: string;
  projId?: number;
  requestCount?: number;
  usage?: unknown;
  usageByModel?: unknown;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Poll GET .../turns/{turnId}/usage until a non-null usage sample appears.
 * Returns null if still empty after attempts (never invents tokens). Author: kejiqing
 */
export async function fetchTurnUsageUntilReady(opts: {
  gatewayBase: string;
  sessionId: string;
  turnId: string;
  projId: number;
  attempts?: number;
  intervalMs?: number;
}): Promise<TurnUsageAggregate | null> {
  const attempts = opts.attempts ?? 8;
  const intervalMs = opts.intervalMs ?? 400;
  const path = `/v1/sessions/${encodeURIComponent(opts.sessionId)}/turns/${encodeURIComponent(opts.turnId)}/usage?proj_id=${encodeURIComponent(String(opts.projId))}`;
  let last: TurnUsageAggregate | null = null;
  for (let i = 0; i < attempts; i++) {
    try {
      const res = await proxyHttp<TurnUsageApiResponse>(
        opts.gatewayBase,
        "GET",
        path
      );
      last = parseTurnUsageAggregate({
        requestCount: res.requestCount ?? 0,
        usage: res.usage ?? null,
        usageByModel: res.usageByModel ?? null,
      });
      if (last?.usage) return last;
    } catch {
      /* keep polling */
    }
    if (i + 1 < attempts) await sleep(intervalMs);
  }
  return last;
}
