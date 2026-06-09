/** Version compare API (camelCase). Author: kejiqing */

export interface ConfigFieldChange {
  field: string;
  kind: string;
  detail: string;
}

/** Expanded revision JSON returned by compare (mergeable top-level keys). */
export interface ProjectConfigDocument {
  contentRev?: string;
  note?: string | null;
  claudeMd?: string | null;
  rulesJson?: unknown[];
  skillsJson?: unknown[];
  mcpServersJson?: Record<string, unknown>;
  allowedToolsJson?: string[];
}

export type MergePickSide = "from" | "to";

export type MergeableField =
  | "claudeMd"
  | "rulesJson"
  | "skillsJson"
  | "mcpServersJson"
  | "allowedToolsJson";

export const MERGEABLE_FIELDS: MergeableField[] = [
  "claudeMd",
  "rulesJson",
  "skillsJson",
  "mcpServersJson",
  "allowedToolsJson",
];

export interface ProjectConfigCompareResponse {
  projId: number;
  from: string;
  to: string;
  activeContentRev: string;
  same: boolean;
  changes: ConfigFieldChange[];
  fromDocument: ProjectConfigDocument;
  toDocument: ProjectConfigDocument;
}
