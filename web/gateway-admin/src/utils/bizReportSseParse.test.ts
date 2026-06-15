import { describe, expect, it } from "vitest";

import {
  bizReportDeltaText,
  bizReportNumField,
  parseBizReportSseJson,
  reportTextFromBizReportDone,
} from "./bizReportSseParse";

describe("bizReportSseParse", () => {
  it("parses delta text", () => {
    const data = parseBizReportSseJson('{"text":"▸ 进度\\n","seq":1,"serverDeltaMs":12}');
    expect(bizReportDeltaText(data)).toBe("▸ 进度\n");
    expect(bizReportNumField(data, "seq")).toBe(1);
    expect(bizReportNumField(data, "missing")).toBeUndefined();
  });

  it("rejects invalid JSON", () => {
    expect(parseBizReportSseJson("not-json")).toBeNull();
  });

  it("extracts report from done payload reportText", () => {
    const text = reportTextFromBizReportDone({
      reportText: '{"message":"报告正文"}',
    });
    expect(text).toContain("报告正文");
  });

  it("extracts report from done payload reportJson.message", () => {
    const text = reportTextFromBizReportDone({
      reportJson: { message: "来自 outputJson 的全文" },
    });
    expect(text).toBe("来自 outputJson 的全文");
  });
});
