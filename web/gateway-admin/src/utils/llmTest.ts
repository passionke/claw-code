/** LLM connectivity test API. Author: kejiqing */

import { proxyHttp } from "../api/client";
import type { LlmTestRequest, LlmTestResponse, ThinkingMode } from "../types/llmTest";

export function thinkingModeToApi(mode: ThinkingMode): boolean | undefined {
  if (mode === "on") return true;
  if (mode === "off") return false;
  return undefined;
}

/** Gateway may omit empty vec fields; normalize before render. */
export function normalizeLlmTestResponse(raw: unknown): LlmTestResponse {
  const r = (raw && typeof raw === "object" ? raw : {}) as Partial<LlmTestResponse>;
  return {
    ok: !!r.ok,
    status: typeof r.status === "string" ? r.status : "unknown",
    modelId: typeof r.modelId === "string" ? r.modelId : "",
    modelName: typeof r.modelName === "string" ? r.modelName : "",
    upstreamUrl: typeof r.upstreamUrl === "string" ? r.upstreamUrl : "",
    responseText: typeof r.responseText === "string" ? r.responseText : undefined,
    thinkingText: typeof r.thinkingText === "string" ? r.thinkingText : undefined,
    usage:
      r.usage && typeof r.usage === "object"
        ? {
            inputTokens:
              typeof (r.usage as LlmTestResponse["usage"])?.inputTokens === "number"
                ? (r.usage as LlmTestResponse["usage"])!.inputTokens
                : 0,
            outputTokens:
              typeof (r.usage as LlmTestResponse["usage"])?.outputTokens === "number"
                ? (r.usage as LlmTestResponse["usage"])!.outputTokens
                : 0,
            totalTokens:
              typeof (r.usage as LlmTestResponse["usage"])?.totalTokens === "number"
                ? (r.usage as LlmTestResponse["usage"])!.totalTokens
                : 0,
          }
        : undefined,
    thinkingEnabled:
      typeof r.thinkingEnabled === "boolean" ? r.thinkingEnabled : undefined,
    temperature: typeof r.temperature === "number" ? r.temperature : undefined,
    topP: typeof r.topP === "number" ? r.topP : undefined,
    warnings: Array.isArray(r.warnings) ? r.warnings : [],
    errors: Array.isArray(r.errors) ? r.errors : [],
    durationMs: typeof r.durationMs === "number" ? r.durationMs : 0,
    hint: typeof r.hint === "string" ? r.hint : "",
  };
}

export async function testLlmModel(
  gatewayBase: string,
  req: LlmTestRequest
): Promise<LlmTestResponse> {
  const raw = await proxyHttp<unknown>(
    gatewayBase,
    "POST",
    "/v1/gateway/global-settings/llm-models/test",
    req
  );
  return normalizeLlmTestResponse(raw);
}
