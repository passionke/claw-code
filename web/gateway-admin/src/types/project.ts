/** Gateway project / config types (camelCase API). Author: kejiqing */

/** First-turn solve preflight (`project_config.solve_preflight_json`). Author: kejiqing */
export interface SolvePreflightJson {
  kind: "none" | "sqlbot_mcp_start" | string;
}

export interface GitSyncJson {
  enabled?: boolean;
  gitUrl?: string;
  gitRef?: string;
  gitPatId?: string;
  gitToken?: string;
  gitTokenSet?: boolean;
  lastPushAtMs?: number;
  lastPushCommitId?: string;
  lastPushError?: string;
  lastPushOk?: boolean;
  configured?: boolean;
}

export interface ProjectListItem {
  dsId: number;
  contentRev?: string;
  draftOpen?: boolean;
  updatedAtMs?: number;
  skillsCountDb?: number;
  claudeInDb?: boolean;
  environmentPrepared?: boolean;
  dbSyncedToDisk?: boolean;
  workDirPresent?: boolean;
  gitSync?: GitSyncJson;
}

export interface ProjectConfig {
  dsId: number;
  contentRev: string;
  stableContentRev?: string;
  draftOpen?: boolean;
  updatedAtMs?: number;
  rulesJson: RuleJsonItem[];
  mcpServersJson: Record<string, unknown>;
  skillsJson: SkillJsonItem[];
  allowedToolsJson: string[];
  claudeMd?: string | null;
  gitSyncJson?: GitSyncJson;
  solvePreflightJson?: SolvePreflightJson;
}

export interface SkillJsonItem {
  skillName: string;
  skillContent: string;
}

export interface RuleJsonItem {
  ruleId?: string;
  ruleTitle?: string;
  ruleScope?: string;
  relativePath?: string;
  content?: string;
}

export interface VersionEntry {
  contentRev: string;
  createdAtMs: number;
  isDraft?: boolean;
  note?: string;
  isActive: boolean;
  claudeInDb: boolean;
  skillsCountDb: number;
  rulesCountDb?: number;
  mcpServersCountDb?: number;
}

export interface VersionsResponse {
  dsId: number;
  activeContentRev: string;
  appliedContentRev?: string;
  draftOpen: boolean;
  versions: VersionEntry[];
}

export interface ToolCatalogEntry {
  name: string;
  description?: string;
  source?: string;
}

export interface RuleEditorItem {
  ruleId: string;
  ruleTitle: string;
  ruleScope: string;
  ruleContent: string;
}

export interface SkillRow {
  skill_name: string;
  skill_content?: string;
}
