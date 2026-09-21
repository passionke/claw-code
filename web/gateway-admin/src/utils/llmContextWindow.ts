/** Match Admin LLM cards by Base URL + model ID. Author: kejiqing */

export function normalizeLlmEndpointUrl(url: string): string {
  return url.trim().replace(/\/+$/, "");
}

export function sameLlmEndpoint(
  leftUrl: string,
  leftModelId: string,
  rightUrl: string,
  rightModelId: string
): boolean {
  return (
    normalizeLlmEndpointUrl(leftUrl) === normalizeLlmEndpointUrl(rightUrl) &&
    leftUrl.trim() !== "" &&
    leftModelId.trim() === rightModelId.trim() &&
    leftModelId.trim() !== ""
  );
}

export function savedContextWindowForEndpoint(
  models: Array<{
    baseModelUrl: string;
    modelName: string;
    contextWindowTokens?: number | null;
  }>,
  baseModelUrl: string,
  modelName: string
): number | undefined {
  for (const row of models) {
    if (!sameLlmEndpoint(row.baseModelUrl, row.modelName, baseModelUrl, modelName)) {
      continue;
    }
    const n = row.contextWindowTokens;
    if (typeof n === "number" && Number.isFinite(n) && n > 0) {
      return Math.floor(n);
    }
  }
  return undefined;
}
