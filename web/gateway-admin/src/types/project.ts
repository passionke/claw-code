/** Gateway project / config types (camelCase API). Author: kejiqing */

import type { SolvePreflightJson } from "./preflight";

export type { SolvePreflightJson } from "./preflight";

/** Solve orchestration pipeline (`project_config.solve_orchestration_json`). Author: kejiqing */
export interface SolveOrchestrationJson {
  kind: "single_turn" | "multi_agent_analysis" | string;
  plannerMaxIter?: number;
  writerMaxIter?: number;
  queryConcurrency?: number;
  narratorModel?: string | null;
  narratorThrottleMs?: number;
}

/** Per-turn language inference (`project_config.language_pipeline_json`). Author: kejiqing */
export interface LanguagePipelineJson {
  languageInferencePrompt?: string;
  languageInferencePriorTurns?: number;
  languageInferencePriorMaxChars?: number;
}

export interface GitRemoteJson {
  id?: string;
  gitUrl?: string;
  gitRef?: string;
  gitPatId?: string | null;
  gitToken?: string;
  gitTokenSet?: boolean;
  destRel?: string;
  lastPullAtMs?: number;
  lastPullCommitId?: string;
  lastPullError?: string;
}

export interface GitSyncJson {
  enabled?: boolean;
  remotes?: GitRemoteJson[];
  /** Legacy single-repo fields (read as remotes[0]). Author: kejiqing */
  gitUrl?: string;
  gitRef?: string;
  gitPatId?: string | null;
  gitToken?: string;
  gitTokenSet?: boolean;
  lastPullAtMs?: number;
  lastPullCommitId?: string;
  lastPullError?: string;
  lastPullOk?: boolean;
  configured?: boolean;
  remoteCount?: number;
}

export interface ProjectListItem {
  projId: number;
  projectRole?: string;
  projectCode?: string;
  projectDescription?: string;
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

export interface PromptLimitsJson {
  /** Per `CLAUDE.md` / rule file cap in system prompt (Unicode chars). Author: kejiqing */
  instructionFileMaxChars?: number;
  /** Combined cap per `# Claude instructions` or `# Project rules` section. Author: kejiqing */
  instructionTotalMaxChars?: number;
}

import type { WorkerProfileJson } from "./landlock";

export type { WorkerProfileJson } from "./landlock";

export interface ProjectConfig {
  projId: number;
  projectRole?: string;
  projectCode?: string;
  projectDescription?: string;
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
  solveOrchestrationJson?: SolveOrchestrationJson;
  languagePipelineJson?: LanguagePipelineJson;
  /** Allowed extraSession business keys for this ds. Author: kejiqing */
  extraSessionFieldsJson?: string[];
  /** Instruction truncation budgets → `.claw/settings.json`. Author: kejiqing */
  promptLimitsJson?: PromptLimitsJson;
  /** Pool worker strict/relaxed (`project_config.worker_profile_json`). Author: kejiqing */
  workerProfileJson?: WorkerProfileJson;
  /** Custom env injected only at warm-proj create (`project_config.worker_env_json`). Author: kejiqing */
  workerEnvJson?: Record<string, string>;
  /** Knowledge-base source mappings (`project_config.kb_sources_json`). Author: kejiqing */
  kbSourcesJson?: KbSourceItem[];
  /** Project default agent loop max iterations; null/omit = cluster CLAW_MAX_ITERATIONS. Author: kejiqing */
  maxIterations?: number | null;
}

export interface KbSourceItem {
  sourceType?: string;
  sourceUrl?: string;
  folderId?: string;
  targetRelPath?: string;
  enabled?: boolean;
}

export interface DelegateTargetRow {
  initiatorProjId: number;
  targetProjId: number;
  enabled: boolean;
  label?: string;
  capabilityHint?: string;
  createdAtMs: number;
  updatedAtMs: number;
}

export interface DelegateTargetsResponse {
  initiatorProjId: number;
  targets: DelegateTargetRow[];
}

export interface SkillJsonItem {
  skillName: string;
  skillContent: string;
  /** false = saved in DB but not materialized to solve. Author: kejiqing */
  enabled?: boolean;
  /** Base64 tar/tgz when multi-file package is the source of truth. Author: kejiqing */
  skillArchive?: string;
  skillArchiveFormat?: "tar" | "tgz" | string;
}

export interface RuleJsonItem {
  ruleId?: string;
  ruleTitle?: string;
  ruleScope?: string;
  relativePath?: string;
  content?: string;
  enabled?: boolean;
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
  projId: number;
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
  enabled?: boolean;
}

export interface SkillRow {
  skill_name: string;
  skill_content?: string;
  enabled?: boolean;
  has_archive?: boolean;
}
