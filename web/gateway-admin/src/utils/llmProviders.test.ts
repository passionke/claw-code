import { describe, expect, it } from "vitest";
import {
  findLlmPresetByEndpoint,
  LLM_PROVIDER_CUSTOM_ID,
  matchLlmPreset,
  parseLlmProviderCsv,
} from "./llmProviders";

describe("parseLlmProviderCsv", () => {
  it("parses header and rows", () => {
    const rows = parseLlmProviderCsv(
      "preset_id,provider_label,base_model_url,model_id,display_name\n" +
        "a,DeepSeek,https://api.deepseek.com/v1,deepseek-v4-flash,DeepSeek V4 Flash\n"
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].presetId).toBe("a");
    expect(rows[0].baseModelUrl).toBe("https://api.deepseek.com/v1");
  });

  it("matchLlmPreset matches trimmed url", () => {
    const rows = parseLlmProviderCsv(
      "preset_id,provider_label,base_model_url,model_id,display_name\n" +
        "x,Label,https://api.example.com/v1,model-a,Name\n"
    );
    expect(matchLlmPreset(rows, "https://api.example.com/v1/", "model-a")).toEqual(rows[0]);
    expect(matchLlmPreset(rows, "https://other", "model-a")).toBeUndefined();
  });

  it("custom id constant", () => {
    expect(LLM_PROVIDER_CUSTOM_ID).toBe("custom");
  });
});
