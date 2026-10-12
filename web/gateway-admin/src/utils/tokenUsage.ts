/** Session/turn token usage helpers for playground ChatPage. Author: kejiqing */

import type {
  OpenAiStyleUsage,
  TurnUsageAggregate,
  UsageByModelRow,
} from "../types/chat";

function asNonNegInt(v: unknown): number | null {
  if (typeof v !== "number" || !Number.isFinite(v) || v < 0) return null;
  return Math.trunc(v);
}

/** Parse OpenAI-style usage object; null when missing or not a real sample. Author: kejiqing */
export function parseOpenAiStyleUsage(raw: unknown): OpenAiStyleUsage | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  const prompt = asNonNegInt(o.prompt_tokens);
  const completion = asNonNegInt(o.completion_tokens);
  const total = asNonNegInt(o.total_tokens);
  if (prompt == null || completion == null || total == null) return null;
  const details = o.prompt_tokens_details;
  let prompt_tokens_details: { cached_tokens?: number } | undefined;
  if (details && typeof details === "object" && !Array.isArray(details)) {
    const cached = asNonNegInt((details as Record<string, unknown>).cached_tokens);
    if (cached != null) prompt_tokens_details = { cached_tokens: cached };
  }
  return {
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: total,
    ...(prompt_tokens_details ? { prompt_tokens_details } : {}),
  };
}

function parseUsageByModelRow(raw: unknown): UsageByModelRow | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  const model = typeof o.model === "string" && o.model.trim() ? o.model : null;
  const prompt = asNonNegInt(o.prompt_tokens);
  const completion = asNonNegInt(o.completion_tokens);
  const total = asNonNegInt(o.total_tokens);
  if (!model || prompt == null || completion == null || total == null) return null;
  return {
    model,
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: total,
  };
}

/** Normalize API / SSE payload into TurnUsageAggregate; null when no usage sample. Author: kejiqing */
export function parseTurnUsageAggregate(raw: unknown): TurnUsageAggregate | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  // GET .../turns wraps { requestCount, usage, usageByModel }
  if ("usage" in o || "usageByModel" in o || "requestCount" in o) {
    const usage = parseOpenAiStyleUsage(o.usage);
    const byModelRaw = o.usageByModel;
    let usageByModel: UsageByModelRow[] | null = null;
    if (Array.isArray(byModelRaw)) {
      const rows = byModelRaw
        .map(parseUsageByModelRow)
        .filter((r): r is UsageByModelRow => r != null);
      usageByModel = rows.length ? rows : null;
    }
    if (!usage) {
      return null;
    }
    const requestCount =
      asNonNegInt(o.requestCount) ??
      (usageByModel?.length ? usageByModel.length : 1);
    return { requestCount, usage, usageByModel };
  }
  // Bare OpenAI usage object
  const usage = parseOpenAiStyleUsage(raw);
  if (!usage) return null;
  return { requestCount: 1, usage, usageByModel: null };
}

/** Build aggregate from response.completed (usage + nerogate.usageByModel). Author: kejiqing */
export function turnUsageFromCompletedResponse(
  response: Record<string, unknown> | null
): TurnUsageAggregate | null {
  if (!response) return null;
  const usage = parseOpenAiStyleUsage(response.usage);
  if (!usage) return null;
  const nerogate = response.nerogate;
  let usageByModel: UsageByModelRow[] | null = null;
  if (nerogate && typeof nerogate === "object" && !Array.isArray(nerogate)) {
    const byModel = (nerogate as Record<string, unknown>).usageByModel;
    if (Array.isArray(byModel)) {
      const rows = byModel
        .map(parseUsageByModelRow)
        .filter((r): r is UsageByModelRow => r != null);
      usageByModel = rows.length ? rows : null;
    }
  }
  return {
    requestCount: usageByModel?.length ? usageByModel.length : 1,
    usage,
    usageByModel,
  };
}

export interface SessionUsageSum {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  turnCountWithUsage: number;
}

/** Sum non-null turn usages; returns null when no turn has a sample. Author: kejiqing */
export function sumSessionUsage(
  aggregates: Array<TurnUsageAggregate | null | undefined>
): SessionUsageSum | null {
  let prompt = 0;
  let completion = 0;
  let total = 0;
  let n = 0;
  for (const a of aggregates) {
    if (!a?.usage) continue;
    prompt += a.usage.prompt_tokens;
    completion += a.usage.completion_tokens;
    total += a.usage.total_tokens;
    n += 1;
  }
  if (n === 0) return null;
  return {
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: total,
    turnCountWithUsage: n,
  };
}

/** Compact label for cards; null when no usage → UI shows "—". Author: kejiqing */
export function formatTokenUsage(usage: OpenAiStyleUsage | null | undefined): string | null {
  if (!usage) return null;
  return `${usage.prompt_tokens} / ${usage.completion_tokens} / ${usage.total_tokens}`;
}

/** Tooltip lines for usageByModel. Author: kejiqing */
export function formatUsageByModelTooltip(
  rows: UsageByModelRow[] | null | undefined
): string | null {
  if (!rows?.length) return null;
  return rows
    .map(
      (r) =>
        `${r.model}: in ${r.prompt_tokens} / out ${r.completion_tokens} / total ${r.total_tokens}`
    )
    .join("\n");
}
