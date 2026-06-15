/** Pure view-state helpers for ChatTurnCard live/history report UI. Author: kejiqing */

import { isEffectiveHistoryTurnView, isTerminalTurnStatus } from "./turnViewMode";

export type TurnCardReportViewInput = {
  viewMode: "live" | "history" | undefined;
  status: string;
  historyReport: string;
  streamText: string;
  streamLive: boolean;
  errorText: string;
};

export type TurnCardReportViewState = {
  historyMode: boolean;
  reportText: string;
  reportVisible: boolean;
  reportStreaming: boolean;
  showStreamingPlaceholder: boolean;
  showHistoryLoadingPlaceholder: boolean;
  shouldConnectLiveSse: boolean;
};

/** Whether the card should open `biz_advice_report?stream=true` SSE. Author: kejiqing */
export function shouldConnectLiveReportSse(
  viewMode: "live" | "history" | undefined,
  status: string
): boolean {
  return !isEffectiveHistoryTurnView(viewMode, status);
}

export function resolveTurnReportText(
  historyMode: boolean,
  historyReport: string,
  streamText: string
): string {
  return historyMode ? historyReport : streamText;
}

export function isTurnReportStreaming(historyMode: boolean, streamLive: boolean): boolean {
  return historyMode ? false : streamLive;
}

export function shouldShowStreamingPlaceholder(input: {
  historyMode: boolean;
  status: string;
  reportStreaming: boolean;
  reportVisible: boolean;
  errorText: string;
}): boolean {
  return (
    !input.historyMode &&
    !isTerminalTurnStatus(input.status) &&
    input.reportStreaming &&
    !input.reportVisible &&
    !input.errorText.trim()
  );
}

export function shouldShowHistoryLoadingPlaceholder(input: {
  historyMode: boolean;
  historyReportLoading: boolean;
  reportVisible: boolean;
  errorText: string;
}): boolean {
  return (
    input.historyMode &&
    input.historyReportLoading &&
    !input.reportVisible &&
    !input.errorText.trim()
  );
}

/** Merge streamed text into history when poll flips to terminal before DB fetch. Author: kejiqing */
export function mergeStreamedIntoHistory(
  historyMode: boolean,
  historyReport: string,
  streamText: string
): string | null {
  if (!historyMode || historyReport.trim()) return null;
  const streamed = streamText.trim();
  return streamed || null;
}

export function deriveTurnCardReportView(
  input: TurnCardReportViewInput & {
    historyReportLoading: boolean;
  }
): TurnCardReportViewState {
  const historyMode = isEffectiveHistoryTurnView(input.viewMode, input.status);
  const reportText = resolveTurnReportText(historyMode, input.historyReport, input.streamText);
  const reportVisible = reportText.length > 0;
  const reportStreaming = isTurnReportStreaming(historyMode, input.streamLive);
  return {
    historyMode,
    reportText,
    reportVisible,
    reportStreaming,
    showStreamingPlaceholder: shouldShowStreamingPlaceholder({
      historyMode,
      status: input.status,
      reportStreaming,
      reportVisible,
      errorText: input.errorText,
    }),
    showHistoryLoadingPlaceholder: shouldShowHistoryLoadingPlaceholder({
      historyMode,
      historyReportLoading: input.historyReportLoading,
      reportVisible,
      errorText: input.errorText,
    }),
    shouldConnectLiveSse: shouldConnectLiveReportSse(input.viewMode, input.status),
  };
}
