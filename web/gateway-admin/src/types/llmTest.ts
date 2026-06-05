/** `POST /v1/gateway/global-settings/llm-models/test`. Author: kejiqing */

export type ThinkingMode = "default" | "on" | "off";

export interface LlmTestRequest {
  modelId: string;
  prompt?: string;
  /** Omit for provider default; send boolean for explicit on/off. */
  thinkingEnabled?: boolean;
  temperature?: number;
  topP?: number;
  maxTokens?: number;
  frequencyPenalty?: number;
  presencePenalty?: number;
  reasoningEffort?: string;
}

export interface LlmTestUsage {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

export interface LlmTestResponse {
  ok: boolean;
  status: string;
  modelId: string;
  modelName: string;
  upstreamUrl: string;
  responseText?: string;
  thinkingText?: string;
  usage?: LlmTestUsage;
  thinkingEnabled?: boolean;
  temperature?: number;
  topP?: number;
  warnings: string[];
  errors: string[];
  durationMs: number;
  hint: string;
}
