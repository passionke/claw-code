import type { ProjectConfig, RuleJsonItem, SkillJsonItem } from "../types/project";
import { slugRuleTitle } from "./rules";
import type {
  MergeableField,
  MergePickSide,
  ProjectConfigDocument,
} from "../types/compare";
import { MERGEABLE_FIELDS } from "../types/compare";
import type { ProjectConfigCompareResponse } from "../types/compare";
import { formatVersionTime } from "./versionDisplay";

export const EMPTY_PROJECT_CONFIG_DOCUMENT: ProjectConfigDocument = {
  claudeMd: null,
  rulesJson: [],
  skillsJson: [],
  mcpServersJson: {},
  allowedToolsJson: [],
};

/** Labels for merge radios (Git-style: base vs incoming). Author: kejiqing */
export function mergeSideLabels(
  fromRev: string,
  toRev: string,
  fromMs?: number | null,
  toMs?: number | null
) {
  return {
    from: `基准版 · ${formatVersionTime(fromRev, fromMs)}`,
    to: `对照版 · ${formatVersionTime(toRev, toMs)}`,
    fromShort: "基准版",
    toShort: "对照版",
  };
}

export function hasCompareDocuments(
  r: ProjectConfigCompareResponse | null | undefined
): r is ProjectConfigCompareResponse & {
  fromDocument: ProjectConfigDocument;
  toDocument: ProjectConfigDocument;
} {
  if (!r) return false;
  const from = r.fromDocument;
  const to = r.toDocument;
  return (
    from != null &&
    typeof from === "object" &&
    to != null &&
    typeof to === "object"
  );
}

export function stableStringify(doc: ProjectConfigDocument | undefined): string {
  return JSON.stringify(doc ?? EMPTY_PROJECT_CONFIG_DOCUMENT, null, 2);
}

/** Pretty JSON for L2 entity bodies or arbitrary snapshots. Author: kejiqing */
export function stableStringifyValue(value: unknown): string {
  if (value === undefined) return "";
  return JSON.stringify(value, null, 2);
}

export function fieldChanged(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined,
  field: MergeableField
): boolean {
  const a = from ?? EMPTY_PROJECT_CONFIG_DOCUMENT;
  const b = to ?? EMPTY_PROJECT_CONFIG_DOCUMENT;
  return JSON.stringify(a[field] ?? null) !== JSON.stringify(b[field] ?? null);
}

export function changedFieldsFromSummary(
  r: ProjectConfigCompareResponse
): MergeableField[] {
  const out = new Set<MergeableField>();
  for (const c of r.changes || []) {
    if ((MERGEABLE_FIELDS as readonly string[]).includes(c.field)) {
      out.add(c.field as MergeableField);
    }
  }
  return MERGEABLE_FIELDS.filter((f) => out.has(f));
}

export function defaultFieldPicks(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined
): Record<MergeableField, MergePickSide> {
  const picks = {} as Record<MergeableField, MergePickSide>;
  for (const f of MERGEABLE_FIELDS) {
    picks[f] = fieldChanged(from, to, f) ? "to" : "from";
  }
  return picks;
}

export function defaultPicksLegacy(
  r: ProjectConfigCompareResponse
): Record<MergeableField, MergePickSide> {
  const picks = {} as Record<MergeableField, MergePickSide>;
  const changed = new Set(changedFieldsFromSummary(r));
  for (const f of MERGEABLE_FIELDS) {
    picks[f] = changed.has(f) ? "to" : "from";
  }
  return picks;
}

/** @deprecated use defaultFieldPicks */
export const defaultPicks = defaultFieldPicks;

export interface MergePickState {
  fields: Record<MergeableField, MergePickSide>;
  skills: Record<string, MergePickSide>;
  /** Key = ruleId（与网关 rulesJson 条目一致） */
  rules: Record<string, MergePickSide>;
  /** Key = mcpServers 对象键（serverName） */
  mcps: Record<string, MergePickSide>;
}

export function defaultMergePickState(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined
): MergePickState {
  return {
    fields: defaultFieldPicks(from, to),
    skills: defaultSkillPicks(listSkillDiffs(from, to)),
    rules: defaultRulePicks(listRuleDiffs(from, to)),
    mcps: defaultMcpPicks(listMcpDiffs(from, to)),
  };
}

function parseSkills(raw: unknown[] | undefined): SkillJsonItem[] {
  if (!Array.isArray(raw)) return [];
  const out: SkillJsonItem[] = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const o = item as Record<string, unknown>;
    const skillName = String(o.skillName ?? "").trim();
    if (!skillName) continue;
    out.push({
      skillName,
      skillContent: String(o.skillContent ?? ""),
    });
  }
  return out;
}

function skillMap(skills: SkillJsonItem[]): Map<string, SkillJsonItem> {
  return new Map(skills.map((s) => [s.skillName, s]));
}

export type SkillDiffKind = "added" | "removed" | "modified";

export interface SkillDiffEntry {
  skillName: string;
  kind: SkillDiffKind;
}

export function listSkillDiffs(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined
): SkillDiffEntry[] {
  const fromMap = skillMap(parseSkills(from?.skillsJson));
  const toMap = skillMap(parseSkills(to?.skillsJson));
  const names = new Set([...fromMap.keys(), ...toMap.keys()]);
  const entries: SkillDiffEntry[] = [];
  for (const name of [...names].sort()) {
    const f = fromMap.get(name);
    const t = toMap.get(name);
    if (!f && t) entries.push({ skillName: name, kind: "added" });
    else if (f && !t) entries.push({ skillName: name, kind: "removed" });
    else if (f && t && f.skillContent !== t.skillContent) {
      entries.push({ skillName: name, kind: "modified" });
    }
  }
  return entries;
}

export function defaultSkillPicks(
  entries: SkillDiffEntry[]
): Record<string, MergePickSide> {
  const picks: Record<string, MergePickSide> = {};
  for (const e of entries) {
    picks[e.skillName] = "to";
  }
  return picks;
}

export function mergeSkillsJson(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined,
  skillPicks: Record<string, MergePickSide>
): SkillJsonItem[] {
  const fromMap = skillMap(parseSkills(from?.skillsJson));
  const toMap = skillMap(parseSkills(to?.skillsJson));
  const names = new Set([...fromMap.keys(), ...toMap.keys()]);
  const merged: SkillJsonItem[] = [];
  for (const name of [...names].sort()) {
    const f = fromMap.get(name);
    const t = toMap.get(name);
    const pick = skillPicks[name] ?? "to";
    if (!f && t) {
      if (pick === "to") merged.push(t);
    } else if (f && !t) {
      if (pick === "from") merged.push(f);
    } else if (f && t) {
      if (f.skillContent === t.skillContent) merged.push(f);
      else merged.push(pick === "from" ? f : t);
    }
  }
  return merged;
}

export const SKILL_DIFF_KIND_LABEL: Record<SkillDiffKind, string> = {
  added: "新增",
  removed: "删除",
  modified: "修改",
};

export const RULE_DIFF_KIND_LABEL = SKILL_DIFF_KIND_LABEL;

function parseRules(raw: unknown[] | undefined): RuleJsonItem[] {
  if (!Array.isArray(raw)) return [];
  const out: RuleJsonItem[] = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const o = item as Record<string, unknown>;
    const ruleTitle = String(o.ruleTitle ?? "").trim();
    const ruleId = String(o.ruleId ?? "").trim();
    const relativePath = String(o.relativePath ?? "").trim();
    const key = ruleMergeKey({ ruleId, ruleTitle, relativePath });
    if (!key) continue;
    out.push({
      ruleId: ruleId || key,
      ruleTitle: ruleTitle || ruleId || key,
      ruleScope: String(o.ruleScope ?? "ALWAYS"),
      relativePath: relativePath || `.cursor/rules/${key}.mdc`,
      content: String(o.content ?? ""),
    });
  }
  return out;
}

/** Stable id for merge UI (prefer ruleId). Author: kejiqing */
export function ruleMergeKey(r: {
  ruleId?: string;
  ruleTitle?: string;
  relativePath?: string;
}): string {
  const id = String(r.ruleId ?? "").trim();
  if (id) return id;
  const path = String(r.relativePath ?? "").trim();
  if (path) {
    const base = path.replace(/^.*\//, "").replace(/\.mdc?$/i, "");
    if (base) return base;
  }
  const title = String(r.ruleTitle ?? "").trim();
  if (title) return slugRuleTitle(title);
  return "";
}

export function ruleDisplayName(r: RuleJsonItem): string {
  return (
    String(r.ruleTitle ?? "").trim() ||
    String(r.ruleId ?? "").trim() ||
    ruleMergeKey(r) ||
    "rule"
  );
}

function ruleMap(rules: RuleJsonItem[]): Map<string, RuleJsonItem> {
  const m = new Map<string, RuleJsonItem>();
  for (const r of rules) {
    const k = ruleMergeKey(r);
    if (k) m.set(k, r);
  }
  return m;
}

function ruleBodyEqual(a: RuleJsonItem, b: RuleJsonItem): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export type RuleDiffKind = SkillDiffKind;

export interface RuleDiffEntry {
  ruleKey: string;
  ruleName: string;
  kind: RuleDiffKind;
}

export function listRuleDiffs(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined
): RuleDiffEntry[] {
  const fromMap = ruleMap(parseRules(from?.rulesJson));
  const toMap = ruleMap(parseRules(to?.rulesJson));
  const keys = new Set([...fromMap.keys(), ...toMap.keys()]);
  const entries: RuleDiffEntry[] = [];
  for (const key of [...keys].sort()) {
    const f = fromMap.get(key);
    const t = toMap.get(key);
    const ruleName = ruleDisplayName(f ?? t!);
    if (!f && t) entries.push({ ruleKey: key, ruleName, kind: "added" });
    else if (f && !t) entries.push({ ruleKey: key, ruleName, kind: "removed" });
    else if (f && t && !ruleBodyEqual(f, t)) {
      entries.push({ ruleKey: key, ruleName, kind: "modified" });
    }
  }
  return entries;
}

export function defaultRulePicks(
  entries: RuleDiffEntry[]
): Record<string, MergePickSide> {
  const picks: Record<string, MergePickSide> = {};
  for (const e of entries) {
    picks[e.ruleKey] = "to";
  }
  return picks;
}

export function mergeRulesJson(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined,
  rulePicks: Record<string, MergePickSide>
): RuleJsonItem[] {
  const fromMap = ruleMap(parseRules(from?.rulesJson));
  const toMap = ruleMap(parseRules(to?.rulesJson));
  const keys = new Set([...fromMap.keys(), ...toMap.keys()]);
  const merged: RuleJsonItem[] = [];
  for (const key of [...keys].sort()) {
    const f = fromMap.get(key);
    const t = toMap.get(key);
    const pick = rulePicks[key] ?? "to";
    if (!f && t) {
      if (pick === "to") merged.push(t);
    } else if (f && !t) {
      if (pick === "from") merged.push(f);
    } else if (f && t) {
      if (ruleBodyEqual(f, t)) merged.push(f);
      else merged.push(pick === "from" ? f : t);
    }
  }
  return merged;
}

function parseMcpServers(
  raw: Record<string, unknown> | undefined
): Map<string, Record<string, unknown>> {
  const m = new Map<string, Record<string, unknown>>();
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return m;
  for (const [k, v] of Object.entries(raw)) {
    const name = k.trim();
    if (!name) continue;
    const cfg =
      v && typeof v === "object" && !Array.isArray(v)
        ? (v as Record<string, unknown>)
        : {};
    m.set(name, cfg);
  }
  return m;
}

function mcpConfigEqual(
  a: Record<string, unknown>,
  b: Record<string, unknown>
): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Short label for merge row (type / url / command). Author: kejiqing */
export function mcpDisplayHint(config: Record<string, unknown>): string {
  const url = config.url != null ? String(config.url).trim() : "";
  const cmd = config.command != null ? String(config.command).trim() : "";
  const typ = config.type != null ? String(config.type).trim() : "";
  if (url) return `${typ || "http"} · ${url}`;
  if (cmd) return `stdio · ${cmd}`;
  if (typ) return typ;
  return "";
}

export type McpDiffKind = SkillDiffKind;

export interface McpDiffEntry {
  serverName: string;
  hint: string;
  kind: McpDiffKind;
}

export const MCP_DIFF_KIND_LABEL = SKILL_DIFF_KIND_LABEL;

export function listMcpDiffs(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined
): McpDiffEntry[] {
  const fromMap = parseMcpServers(from?.mcpServersJson);
  const toMap = parseMcpServers(to?.mcpServersJson);
  const names = new Set([...fromMap.keys(), ...toMap.keys()]);
  const entries: McpDiffEntry[] = [];
  for (const name of [...names].sort()) {
    const f = fromMap.get(name);
    const t = toMap.get(name);
    const hint = mcpDisplayHint(t ?? f ?? {});
    if (!f && t) entries.push({ serverName: name, hint, kind: "added" });
    else if (f && !t) entries.push({ serverName: name, hint, kind: "removed" });
    else if (f && t && !mcpConfigEqual(f, t)) {
      entries.push({ serverName: name, hint, kind: "modified" });
    }
  }
  return entries;
}

export function defaultMcpPicks(
  entries: McpDiffEntry[]
): Record<string, MergePickSide> {
  const picks: Record<string, MergePickSide> = {};
  for (const e of entries) {
    picks[e.serverName] = "to";
  }
  return picks;
}

export function mergeMcpServersJson(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined,
  mcpPicks: Record<string, MergePickSide>
): Record<string, unknown> {
  const fromMap = parseMcpServers(from?.mcpServersJson);
  const toMap = parseMcpServers(to?.mcpServersJson);
  const names = new Set([...fromMap.keys(), ...toMap.keys()]);
  const merged: Record<string, unknown> = {};
  for (const name of [...names].sort()) {
    const f = fromMap.get(name);
    const t = toMap.get(name);
    const pick = mcpPicks[name] ?? "to";
    if (!f && t) {
      if (pick === "to") merged[name] = t;
    } else if (f && !t) {
      if (pick === "from") merged[name] = f;
    } else if (f && t) {
      if (mcpConfigEqual(f, t)) merged[name] = f;
      else merged[name] = pick === "from" ? f : t;
    }
  }
  return merged;
}

/** Block-level only; skills/rules/mcps merged per entry. Author: kejiqing */
export const BLOCK_MERGE_FIELDS: MergeableField[] = [
  "claudeMd",
  "allowedToolsJson",
];

export function mergeDocumentsToDraftPatch(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined,
  state: MergePickState | Record<MergeableField, MergePickSide>
): Partial<ProjectConfig> {
  const pickState: MergePickState =
    "fields" in state
      ? state
      : {
          fields: state,
          skills: defaultSkillPicks(listSkillDiffs(from, to)),
          rules: defaultRulePicks(listRuleDiffs(from, to)),
          mcps: defaultMcpPicks(listMcpDiffs(from, to)),
        };
  const fromDoc = from ?? EMPTY_PROJECT_CONFIG_DOCUMENT;
  const toDoc = to ?? EMPTY_PROJECT_CONFIG_DOCUMENT;
  const patch: Partial<ProjectConfig> = {};

  for (const field of BLOCK_MERGE_FIELDS) {
    const side = pickState.fields[field] ?? "to";
    const src = side === "from" ? fromDoc : toDoc;
    switch (field) {
      case "claudeMd":
        patch.claudeMd = src.claudeMd ?? null;
        break;
      case "allowedToolsJson":
        patch.allowedToolsJson = (src.allowedToolsJson ??
          []) as ProjectConfig["allowedToolsJson"];
        break;
    }
  }

  if (fieldChanged(from, to, "skillsJson")) {
    patch.skillsJson = mergeSkillsJson(from, to, pickState.skills);
  } else {
    const side = pickState.fields.skillsJson ?? "from";
    const src = side === "from" ? fromDoc : toDoc;
    patch.skillsJson = parseSkills(src.skillsJson);
  }

  if (fieldChanged(from, to, "rulesJson")) {
    patch.rulesJson = mergeRulesJson(from, to, pickState.rules);
  } else {
    const side = pickState.fields.rulesJson ?? "from";
    const src = side === "from" ? fromDoc : toDoc;
    patch.rulesJson = parseRules(src.rulesJson);
  }

  if (fieldChanged(from, to, "mcpServersJson")) {
    patch.mcpServersJson = mergeMcpServersJson(from, to, pickState.mcps);
  } else {
    const side = pickState.fields.mcpServersJson ?? "from";
    const src = side === "from" ? fromDoc : toDoc;
    patch.mcpServersJson = (src.mcpServersJson ?? {}) as ProjectConfig["mcpServersJson"];
  }

  return patch;
}

export const MERGE_FIELD_LABELS: Record<MergeableField, string> = {
  claudeMd: "CLAUDE.md",
  rulesJson: "Rules",
  skillsJson: "Skills",
  mcpServersJson: "MCP",
  allowedToolsJson: "Tools",
};
