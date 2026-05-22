import type { ProjectConfig } from "../types/project";
import type {
  MergeableField,
  MergePickSide,
  ProjectConfigDocument,
} from "../types/compare";
import { MERGEABLE_FIELDS } from "../types/compare";
import type { ProjectConfigCompareResponse } from "../types/compare";

export const EMPTY_PROJECT_CONFIG_DOCUMENT: ProjectConfigDocument = {
  claudeMd: null,
  rulesJson: [],
  skillsJson: [],
  mcpServersJson: {},
  allowedToolsJson: [],
};

/** New gateway compare includes expanded JSON; old gateway only returns `changes`. Author: kejiqing */
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

export function defaultPicks(
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

/** Build draft PUT patch from per-field picks (`from` = left / 已发布侧). Author: kejiqing */
export function mergeDocumentsToDraftPatch(
  from: ProjectConfigDocument | undefined,
  to: ProjectConfigDocument | undefined,
  picks: Record<MergeableField, MergePickSide>
): Partial<ProjectConfig> {
  const fromDoc = from ?? EMPTY_PROJECT_CONFIG_DOCUMENT;
  const toDoc = to ?? EMPTY_PROJECT_CONFIG_DOCUMENT;
  const patch: Partial<ProjectConfig> = {};
  for (const field of MERGEABLE_FIELDS) {
    const side = picks[field] ?? "to";
    const src = side === "from" ? fromDoc : toDoc;
    switch (field) {
      case "claudeMd":
        patch.claudeMd = src.claudeMd ?? null;
        break;
      case "rulesJson":
        patch.rulesJson = (src.rulesJson ?? []) as ProjectConfig["rulesJson"];
        break;
      case "skillsJson":
        patch.skillsJson = (src.skillsJson ?? []) as ProjectConfig["skillsJson"];
        break;
      case "mcpServersJson":
        patch.mcpServersJson = (src.mcpServersJson ??
          {}) as ProjectConfig["mcpServersJson"];
        break;
      case "allowedToolsJson":
        patch.allowedToolsJson = (src.allowedToolsJson ??
          []) as ProjectConfig["allowedToolsJson"];
        break;
    }
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
