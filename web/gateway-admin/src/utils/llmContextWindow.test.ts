import { describe, expect, it } from "vitest";
import {
  savedContextWindowForEndpoint,
  sameLlmEndpoint,
} from "./llmContextWindow";

describe("sameLlmEndpoint", () => {
  it("matches when baseUrl and model id are the same after trimming slash", () => {
    expect(
      sameLlmEndpoint(
        "https://api.example.com/v1/",
        "qwen3.8-max-0902",
        "https://api.example.com/v1",
        "qwen3.8-max-0902"
      )
    ).toBe(true);
  });

  it("does not match same model id on a different gateway", () => {
    expect(
      sameLlmEndpoint(
        "https://api.example.com/v1",
        "qwen3.8-max-0902",
        "https://other.example.com/v1",
        "qwen3.8-max-0902"
      )
    ).toBe(false);
  });
});

describe("savedContextWindowForEndpoint", () => {
  const models = [
    {
      baseModelUrl: "https://litellm.example/v1",
      modelName: "qwen3.8-max-0902",
      contextWindowTokens: 991808,
    },
    {
      baseModelUrl: "https://other.example/v1",
      modelName: "qwen3.8-max-0902",
      contextWindowTokens: 32000,
    },
  ];

  it("brings the window only when both url and model id match", () => {
    expect(
      savedContextWindowForEndpoint(
        models,
        "https://litellm.example/v1/",
        "qwen3.8-max-0902"
      )
    ).toBe(991808);
    expect(
      savedContextWindowForEndpoint(
        models,
        "https://other.example/v1",
        "qwen3.8-max-0902"
      )
    ).toBe(32000);
    expect(
      savedContextWindowForEndpoint(
        models,
        "https://missing.example/v1",
        "qwen3.8-max-0902"
      )
    ).toBeUndefined();
  });
});
