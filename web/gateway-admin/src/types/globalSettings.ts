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
  /** e2b observe singleton; written by `observe-tap-up`, not hand-edited in Admin. */
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

export interface ObserveTapResetResponse {
  tap: ClawTapSettings;
  sandboxId: string;
  liveBaseUrl: string;
  trafficReachable: boolean;
  message?: string;
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
