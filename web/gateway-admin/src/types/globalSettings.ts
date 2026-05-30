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

export interface ClawTapSettings {
  host: string;
  proxyPort: number;
  updatedAtMs: number;
  configured: boolean;
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
  /** Derived from gateway PG URL; read-only. */
  clusterId?: string;
}
