import { describe, expect, it } from "vitest";

import {
  deriveTurnCardReportView,
  mergeStreamedIntoHistory,
  shouldConnectLiveReportSse,
  shouldShowStreamingPlaceholder,
} from "./turnCardReportView";

describe("turnCardReportView — observer window (live + running)", () => {
  it("connects SSE and shows streaming placeholder before first delta", () => {
    const view = deriveTurnCardReportView({
      viewMode: "live",
      status: "running",
      historyReport: "",
      streamText: "",
      streamLive: true,
      errorText: "",
      historyReportLoading: false,
    });
    expect(view.shouldConnectLiveSse).toBe(true);
    expect(view.showStreamingPlaceholder).toBe(true);
    expect(view.reportVisible).toBe(false);
    expect(view.historyMode).toBe(false);
  });

  it("shows streamed text and hides placeholder once deltas arrive", () => {
    const view = deriveTurnCardReportView({
      viewMode: "live",
      status: "running",
      historyReport: "",
      streamText: "▸ 正在撰写文章\n第一章",
      streamLive: true,
      errorText: "",
      historyReportLoading: false,
    });
    expect(view.showStreamingPlaceholder).toBe(false);
    expect(view.reportText).toBe("▸ 正在撰写文章\n第一章");
    expect(view.reportStreaming).toBe(true);
  });
});

describe("turnCardReportView — sender after poll succeeded (thread still live)", () => {
  it("stops live SSE path and never shows streaming placeholder on terminal", () => {
    const view = deriveTurnCardReportView({
      viewMode: "live",
      status: "succeeded",
      historyReport: "全文报告",
      streamText: "",
      streamLive: true,
      errorText: "",
      historyReportLoading: false,
    });
    expect(view.historyMode).toBe(true);
    expect(view.shouldConnectLiveSse).toBe(false);
    expect(view.showStreamingPlaceholder).toBe(false);
    expect(view.reportText).toBe("全文报告");
    expect(view.reportStreaming).toBe(false);
  });

  it("does not flash placeholder when SSE live flag lingers after succeeded", () => {
    expect(
      shouldShowStreamingPlaceholder({
        historyMode: true,
        status: "succeeded",
        reportStreaming: true,
        reportVisible: false,
        errorText: "",
      })
    ).toBe(false);
  });
});

describe("turnCardReportView — history session reload", () => {
  it("loads from history report without live SSE", () => {
    expect(shouldConnectLiveReportSse("history", "succeeded")).toBe(false);
    const view = deriveTurnCardReportView({
      viewMode: "history",
      status: "succeeded",
      historyReport: "持久化报告",
      streamText: "",
      streamLive: false,
      errorText: "",
      historyReportLoading: false,
    });
    expect(view.reportText).toBe("持久化报告");
    expect(view.showHistoryLoadingPlaceholder).toBe(false);
  });
});

describe("mergeStreamedIntoHistory", () => {
  it("copies streamed text when terminal transition beats DB fetch", () => {
    expect(mergeStreamedIntoHistory(true, "", "流式正文")).toBe("流式正文");
    expect(mergeStreamedIntoHistory(true, "已有", "流式")).toBe(null);
    expect(mergeStreamedIntoHistory(false, "", "流式")).toBe(null);
  });
});
