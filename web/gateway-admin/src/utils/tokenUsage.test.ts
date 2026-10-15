/** tokenUsage helpers. Author: kejiqing */

import { describe, expect, it } from "vitest";

import {
  formatTokenUsage,
  formatUsageByModelTooltip,
  parseTurnUsageAggregate,
  sumSessionUsage,
  turnUsageFromCompletedResponse,
  usageMetricParts,
} from "./tokenUsage";

describe("parseTurnUsageAggregate", () => {
  it("parses camelCase turns wrapper", () => {
    const agg = parseTurnUsageAggregate({
      requestCount: 2,
      usage: {
        prompt_tokens: 15,
        completion_tokens: 5,
        total_tokens: 20,
        prompt_tokens_details: { cached_tokens: 4 },
      },
      usageByModel: [
        {
          model: "m1",
          prompt_tokens: 10,
          completion_tokens: 3,
          total_tokens: 13,
        },
        {
          model: "m2",
          prompt_tokens: 5,
          completion_tokens: 2,
          total_tokens: 7,
        },
      ],
    });
    expect(agg?.requestCount).toBe(2);
    expect(agg?.usage?.total_tokens).toBe(20);
    expect(agg?.usage?.prompt_tokens_details?.cached_tokens).toBe(4);
    expect(agg?.usageByModel).toHaveLength(2);
  });

  it("returns null when usage is null", () => {
    expect(
      parseTurnUsageAggregate({
        requestCount: 0,
        usage: null,
        usageByModel: null,
      })
    ).toBeNull();
  });
});

describe("sumSessionUsage", () => {
  it("sums only turns with samples and cache when known", () => {
    const sum = sumSessionUsage([
      {
        requestCount: 1,
        usage: {
          prompt_tokens: 10,
          completion_tokens: 2,
          total_tokens: 12,
          prompt_tokens_details: { cached_tokens: 3 },
        },
      },
      null,
      {
        requestCount: 1,
        usage: { prompt_tokens: 5, completion_tokens: 1, total_tokens: 6 },
      },
    ]);
    expect(sum).toEqual({
      prompt_tokens: 15,
      completion_tokens: 3,
      total_tokens: 18,
      cached_tokens: 3,
      turnCountWithUsage: 2,
    });
  });

  it("returns null when nothing recorded", () => {
    expect(sumSessionUsage([null, undefined])).toBeNull();
  });
});

describe("format helpers", () => {
  it("formats labeled usage with cache", () => {
    expect(
      formatTokenUsage({
        prompt_tokens: 6001,
        completion_tokens: 53,
        total_tokens: 6054,
        prompt_tokens_details: { cached_tokens: 1200 },
      })
    ).toBe("in 6,001 · out 53 · cache 1,200 · total 6,054");
    expect(formatTokenUsage(null)).toBeNull();
    expect(
      usageMetricParts({
        prompt_tokens: 10,
        completion_tokens: 2,
        total_tokens: 12,
      })?.map((p) => p.key)
    ).toEqual(["in", "out", "total"]);
    expect(
      formatUsageByModelTooltip([
        {
          model: "a",
          prompt_tokens: 1,
          completion_tokens: 2,
          total_tokens: 3,
          prompt_tokens_details: { cached_tokens: 1 },
        },
      ])
    ).toBe("a: in 1 · out 2 · cache 1 · total 3");
  });
});

describe("turnUsageFromCompletedResponse", () => {
  it("builds aggregate from response.completed payload", () => {
    const agg = turnUsageFromCompletedResponse({
      usage: {
        prompt_tokens: 8,
        completion_tokens: 2,
        total_tokens: 10,
        prompt_tokens_details: { cached_tokens: 1 },
      },
      nerogate: {
        usageByModel: [
          {
            model: "x",
            prompt_tokens: 8,
            completion_tokens: 2,
            total_tokens: 10,
          },
        ],
      },
    });
    expect(agg?.usage?.total_tokens).toBe(10);
    expect(agg?.usage?.prompt_tokens_details?.cached_tokens).toBe(1);
    expect(agg?.requestCount).toBe(1);
  });
});
