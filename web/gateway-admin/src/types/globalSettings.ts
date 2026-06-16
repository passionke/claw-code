/** Gateway global settings API shapes. Author: kejiqing */

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

export interface ClawTapSettings {
  host: string;
  proxyPort: number;
  livePort: number;
  updatedAtMs: number;
  configured: boolean;
  proxyBaseUrl?: string;
  liveBaseUrl?: string;
  liveSessionUrlTemplate?: string;
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
  adminMcpTokens?: AdminMcpTokenRow[];
  /** Derived from gateway PG URL; read-only. */
  clusterId?: string;
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
