/** Gateway global settings API shapes. Author: kejiqing */

export interface GitPatRow {
  id: string;
  name: string;
  note?: string;
  createdAtMs: number;
  updatedAtMs: number;
  tokenSet: boolean;
}

export interface LlmModelRow {
  id: string;
  name: string;
  baseModelUrl: string;
  modelName: string;
  apiKeySet: boolean;
  active?: boolean;
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

export interface GlobalSettingsResponse {
  updatedAtMs: number;
  gitPats: GitPatRow[];
  llmModels?: LlmModelRow[];
  activeLlmModelId?: string;
  activeLlmAppliedAtMs?: number;
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
