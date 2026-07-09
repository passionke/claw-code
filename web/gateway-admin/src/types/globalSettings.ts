/** Gateway global settings API shapes. Author: kejiqing */

import type { LandlockDsl } from "./landlock";

export interface GitPatRow {
  id: string;
  name: string;
  note?: string;
  createdAtMs: number;
  updatedAtMs: number;
  tokenSet: boolean;
}

export interface ActiveLlmConfig {
  modelId: string;
  name: string;
  baseModelUrl: string;
  modelName: string;
  apiKeySet: boolean;
}

export interface LlmModelRow {
  id: string;
  name: string;
  baseModelUrl: string;
  modelName: string;
  apiKeySet: boolean;
  active?: boolean;
  /** Head revision after last save. */
  currentRev?: string;
  /** Active revision when this row is the current model. */
  activeRev?: string;
  createdAtMs: number;
  updatedAtMs: number;
}

export type ClawTapMode = "local" | "remote";

export interface ClawTapSettings {
  mode: ClawTapMode;
  host: string;
  proxyPort: number;
  livePort?: number;
  updatedAtMs: number;
  configured: boolean;
  proxyBaseUrl?: string;
  liveBaseUrl?: string;
  liveSessionUrlTemplate?: string;
  liveBrowserHostsLine?: string;
  /** e2b observe singleton; gateway lifecycle writes PG. */
  e2bObserveSandboxId?: string;
  /** Live e2b sandbox state from gateway `GET /sandboxes/{id}`. Author: kejiqing */
  e2bObserveSandboxState?: string;
  e2bObserveSandboxRunning?: boolean;
  e2bObserveSandboxEndAtMs?: number;
  e2bObserveSandboxRemainingTtlSecs?: number;
}

export interface PutClawTapSettingsResponse extends ClawTapSettings {
  tapRestart?: {
    attempted: boolean;
    restarted: boolean;
    message?: string;
  };
  message?: string;
}

export interface ClawTapProbeResponse {
  ok: boolean;
  message: string;
  probeUrl: string;
  clusterId?: string;
  dbHost?: string;
  clusterHash?: string;
  localClusterHash?: string;
  clusterMatch?: boolean;
  hashMatch?: boolean;
  latencyMs?: number;
}

export interface E2bNasSettings {
  readOnly: boolean;
  nasHostMount: string;
  e2bNasServer: string;
  e2bNasExport: string;
  configured: boolean;
  gatewayWorkRoot: string;
  nasRootResolved: string;
  layoutActive: boolean;
  pathExists: boolean;
  hasProjTree?: boolean;
}

export interface E2bPlatformSettings {
  readOnly: boolean;
  e2bApiUrl: string;
  e2bSandboxUrl?: string;
  e2bDomain: string;
  apiKeySet: boolean;
  workerStrictTemplate: string;
  workerRelaxedTemplate: string;
  sandboxTimeoutSecs: number;
  configured: boolean;
}

export interface ObserveTapResetResponse {
  tap: ClawTapSettings;
  sandboxId: string;
  liveBaseUrl: string;
  trafficReachable: boolean;
  message?: string;
}

export interface E2bNasApiSettings {
  templateId?: string;
  effectiveTemplateId: string;
  baseUrl?: string;
  sandboxId?: string;
  updatedAtMs: number;
  configured?: boolean;
  running?: boolean;
  reachable?: boolean;
  healthy?: boolean;
  lastCheckedAtMs?: number;
  lastError?: string;
  online: boolean;
}

export interface E2bOvsSettings {
  templateId?: string;
  effectiveTemplateId: string;
  baseUrl?: string;
  sandboxId?: string;
  updatedAtMs: number;
  configured: boolean;
}

export interface E2bObserveTemplateSettings {
  templateId?: string;
  effectiveTemplateId: string;
  updatedAtMs: number;
  configured: boolean;
  baseUrl?: string;
  sandboxId?: string;
  running?: boolean;
  reachable?: boolean;
  healthy?: boolean;
  lastCheckedAtMs?: number;
  lastError?: string;
}

export interface E2bSingletonsStatusResponse {
  nasApi: E2bNasApiSettings;
  ovs: E2bOvsSettings;
  observe: E2bObserveTemplateSettings;
}

export interface PutE2bSingletonTemplatesResponse {
  nasApi: E2bNasApiSettings;
  ovs: E2bOvsSettings;
  observe: E2bObserveTemplateSettings;
}

export interface E2bSingletonActionResponse {
  component: string;
  sandboxId?: string;
  baseUrl?: string;
  trafficReachable: boolean;
  message?: string;
}

export interface E2bTemplateEntry {
  templateId: string;
  aliases: string[];
  imagePresent: boolean;
  image?: string;
  arch?: string;
}

export interface E2bTemplatesListResponse {
  apiUrl: string;
  templates: E2bTemplateEntry[];
}

export interface E2bWorkerSettings {
  templateId?: string;
  /** Strict solve worker pool size per project (PG). Default 4, range 1–16. */
  poolSize?: number;
  updatedAtMs?: number;
}

export interface PutE2bWorkerSettingsInput {
  templateId?: string;
  poolSize?: number;
}

export interface GlobalSettingsResponse {
  updatedAtMs: number;
  gitPats: GitPatRow[];
  llmModels?: LlmModelRow[];
  activeLlmModelId?: string;
  activeLlmModelRev?: string;
  activeLlmAppliedAtMs?: number;
  /** Present only when solve/runtime can load the active LLM. */
  activeLlmConfig?: ActiveLlmConfig;
  clawTap?: ClawTapSettings;
  e2bNas?: E2bNasSettings;
  e2bPlatform?: E2bPlatformSettings;
  e2bNasApi?: E2bNasApiSettings;
  e2bOvs?: E2bOvsSettings;
  e2bObserve?: E2bObserveTemplateSettings;
  e2bWorker?: E2bWorkerSettings;
  adminMcpTokens?: AdminMcpTokenRow[];
  /** Derived from gateway PG URL; read-only. */
  clusterId?: string;
  strictLandlockDefault?: LandlockDsl;
}

export interface AdminMcpTokenRow {
  id: string;
  name: string;
  note?: string;
  kind: "temporary" | "permanent";
  createdAtMs: number;
  expiresAtMs?: number;
  revokedAtMs?: number;
  lastUsedAtMs?: number;
  active: boolean;
  expired: boolean;
}

export interface AdminMcpTokenIssueResponse {
  entry: AdminMcpTokenRow;
  token: string;
  mcpEndpointPath: string;
  mcpTransport: string;
}
