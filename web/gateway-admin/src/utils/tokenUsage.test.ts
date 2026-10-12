/** tokenUsage helpers. Author: kejiqing */

import { describe, expect, it } from "vitest";

import {
  formatTokenUsage,
  formatUsageByModelTooltip,
  parseTurnUsageAggregate,
  sumSessionUsage,
  turnUsageFromCompletedResponse,
} from "./tokenUsage";

describe("parseTurnUsageAggregate", () => {
  it("parses camelCase turns wrapper", () => {
    const agg = parseTurnUsageAggregate({
      requestCount: 2,
      usage: {
        prompt_tokens: 15,
        completion_tokens: 5,
        total_tokens: 20,
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
  it("sums only turns with samples", () => {
    const sum = sumSessionUsage([
      {
        requestCount: 1,
        usage: { prompt_tokens: 10, completion_tokens: 2, total_tokens: 12 },
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
      turnCountWithUsage: 2,
    });
  });

  it("returns null when nothing recorded", () => {
    expect(sumSessionUsage([null, undefined])).toBeNull();
  });
});

describe("format helpers", () => {
  it("formats usage and by-model tooltip", () => {
    expect(
      formatTokenUsage({
        prompt_tokens: 1,
        completion_tokens: 2,
        total_tokens: 3,
      })
    ).toBe("1 / 2 / 3");
    expect(formatTokenUsage(null)).toBeNull();
    expect(
      formatUsageByModelTooltip([
        {
          model: "a",
          prompt_tokens: 1,
          completion_tokens: 2,
          total_tokens: 3,
        },
      ])
    ).toBe("a: in 1 / out 2 / total 3");
  });
});

describe("turnUsageFromCompletedResponse", () => {
  it("builds aggregate from response.completed payload", () => {
    const agg = turnUsageFromCompletedResponse({
      usage: {
        prompt_tokens: 8,
        completion_tokens: 2,
        total_tokens: 10,
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
    expect(agg?.requestCount).toBe(1);
  });
});
