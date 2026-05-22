/** L2 entity revision helpers. Author: kejiqing */

import { proxyHttp } from "../api/client";
export type EntityDomain = "rule" | "skill" | "mcp";
import { parseRuleJsonItem } from "./rules";
import type { RuleEditorItem } from "../types/project";

interface EntityCompareResponse {
  fromBody?: unknown;
  toBody?: unknown;
}

function entityPath(dsId: number, domain: string, entityKey: string, suffix: string) {
  return `/v1/project/config/${dsId}/entities/${domain}/${encodeURIComponent(entityKey)}${suffix}`;
}

/** Fetch one immutable snapshot (compare same rev twice). Author: kejiqing */
export async function fetchEntityRevisionBody(
  gatewayBase: string,
  dsId: number,
  domain: EntityDomain,
  entityKey: string,
  entityRev: string
): Promise<unknown> {
  const r = await proxyHttp<EntityCompareResponse>(
    gatewayBase,
    "GET",
    `${entityPath(dsId, domain, entityKey, "/versions/compare")}?from=${encodeURIComponent(entityRev)}&to=${encodeURIComponent(entityRev)}`
  );
  const body = r.toBody ?? r.fromBody;
  if (body === undefined) {
    throw new Error("网关未返回条目快照 body");
  }
  return body;
}

export function ruleFieldsFromRevisionBody(body: unknown): Pick<RuleEditorItem, "ruleTitle" | "ruleContent"> {
  const item = parseRuleJsonItem(body as import("../types/project").RuleJsonItem);
  return { ruleTitle: item.ruleTitle, ruleContent: item.ruleContent };
}

export function skillContentFromRevisionBody(body: unknown): string {
  if (body && typeof body === "object" && "skillContent" in body) {
    const c = (body as { skillContent?: unknown }).skillContent;
    return typeof c === "string" ? c : "";
  }
  return "";
}

export function mcpConfigJsonFromRevisionBody(body: unknown): string {
  try {
    return `${JSON.stringify(body ?? {}, null, 2)}\n`;
  } catch {
    return "{}\n";
  }
}
