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

function formatCount(n: number): string {
  return n.toLocaleString("en-US");
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

/** Cached tokens; null when `prompt_tokens_details` absent (do not invent 0). Author: kejiqing */
export function cachedTokensOf(usage: OpenAiStyleUsage | null | undefined): number | null {
  if (!usage?.prompt_tokens_details) return null;
  const n = usage.prompt_tokens_details.cached_tokens;
  return typeof n === "number" && Number.isFinite(n) && n >= 0 ? Math.trunc(n) : null;
}

function parseUsageByModelRow(raw: unknown): UsageByModelRow | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  const o = raw as Record<string, unknown>;
  const model = typeof o.model === "string" && o.model.trim() ? o.model : null;
  const prompt = asNonNegInt(o.prompt_tokens);
  const completion = asNonNegInt(o.completion_tokens);
  const total = asNonNegInt(o.total_tokens);
  if (!model || prompt == null || completion == null || total == null) return null;
  const details = o.prompt_tokens_details;
  let prompt_tokens_details: { cached_tokens?: number } | undefined;
  if (details && typeof details === "object" && !Array.isArray(details)) {
    const cached = asNonNegInt((details as Record<string, unknown>).cached_tokens);
    if (cached != null) prompt_tokens_details = { cached_tokens: cached };
  }
  return {
    model,
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: total,
    ...(prompt_tokens_details ? { prompt_tokens_details } : {}),
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
  /** Sum of cached_tokens when present; null if no turn reported cache. Author: kejiqing */
  cached_tokens: number | null;
  turnCountWithUsage: number;
}

/** Sum non-null turn usages; returns null when no turn has a sample. Author: kejiqing */
export function sumSessionUsage(
  aggregates: Array<TurnUsageAggregate | null | undefined>
): SessionUsageSum | null {
  let prompt = 0;
  let completion = 0;
  let total = 0;
  let cached = 0;
  let cachedKnown = false;
  let n = 0;
  for (const a of aggregates) {
    if (!a?.usage) continue;
    prompt += a.usage.prompt_tokens;
    completion += a.usage.completion_tokens;
    total += a.usage.total_tokens;
    const c = cachedTokensOf(a.usage);
    if (c != null) {
      cached += c;
      cachedKnown = true;
    }
    n += 1;
  }
  if (n === 0) return null;
  return {
    prompt_tokens: prompt,
    completion_tokens: completion,
    total_tokens: total,
    cached_tokens: cachedKnown ? cached : null,
    turnCountWithUsage: n,
  };
}

export type UsageMetricKey = "in" | "out" | "cache" | "total";

export interface UsageMetricPart {
  key: UsageMetricKey;
  label: string;
  value: number;
  display: string;
}

/**
 * Labeled metric parts for footer UI.
 * Always includes in/out/total; cache when field present (incl. 0). Author: kejiqing
 */
export function usageMetricParts(
  usage: OpenAiStyleUsage | null | undefined
): UsageMetricPart[] | null {
  if (!usage) return null;
  const cached = cachedTokensOf(usage);
  const parts: UsageMetricPart[] = [
    {
      key: "in",
      label: "in",
      value: usage.prompt_tokens,
      display: formatCount(usage.prompt_tokens),
    },
    {
      key: "out",
      label: "out",
      value: usage.completion_tokens,
      display: formatCount(usage.completion_tokens),
    },
  ];
  if (cached != null) {
    parts.push({
      key: "cache",
      label: "cache",
      value: cached,
      display: formatCount(cached),
    });
  }
  parts.push({
    key: "total",
    label: "total",
    value: usage.total_tokens,
    display: formatCount(usage.total_tokens),
  });
  return parts;
}

/** Compact one-line label (toolbar / a11y); includes cache when known. Author: kejiqing */
export function formatTokenUsage(usage: OpenAiStyleUsage | null | undefined): string | null {
  const parts = usageMetricParts(usage);
  if (!parts) return null;
  return parts.map((p) => `${p.label} ${p.display}`).join(" · ");
}

/** Session sum → OpenAiStyleUsage for formatters. Author: kejiqing */
export function openAiUsageFromSessionSum(sum: SessionUsageSum): OpenAiStyleUsage {
  return {
    prompt_tokens: sum.prompt_tokens,
    completion_tokens: sum.completion_tokens,
    total_tokens: sum.total_tokens,
    ...(sum.cached_tokens != null
      ? { prompt_tokens_details: { cached_tokens: sum.cached_tokens } }
      : {}),
  };
}

/** Tooltip lines for usageByModel. Author: kejiqing */
export function formatUsageByModelTooltip(
  rows: UsageByModelRow[] | null | undefined
): string | null {
  if (!rows?.length) return null;
  return rows
    .map((r) => {
      const cached = cachedTokensOf(r);
      const cacheBit = cached != null ? ` · cache ${formatCount(cached)}` : "";
      return `${r.model}: in ${formatCount(r.prompt_tokens)} · out ${formatCount(r.completion_tokens)}${cacheBit} · total ${formatCount(r.total_tokens)}`;
    })
    .join("\n");
}
