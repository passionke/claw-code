import { describe, expect, it } from "vitest";

import {
  isEffectiveHistoryTurnView,
  isHistoryTurnView,
  isTerminalTurnStatus,
  turnViewModeForStatus,
} from "./turnViewMode";

describe("turnViewMode", () => {
  it("marks terminal statuses", () => {
    expect(isTerminalTurnStatus("succeeded")).toBe(true);
    expect(isTerminalTurnStatus("failed")).toBe(true);
    expect(isTerminalTurnStatus("cancelled")).toBe(true);
    expect(isTerminalTurnStatus("running")).toBe(false);
    expect(isTerminalTurnStatus("queued")).toBe(false);
  });

  it("maps list API view mode from status", () => {
    expect(turnViewModeForStatus("running")).toBe("live");
    expect(turnViewModeForStatus("succeeded")).toBe("history");
  });

  it("history view only when thread item is history and status terminal", () => {
    expect(isHistoryTurnView("history", "succeeded")).toBe(true);
    expect(isHistoryTurnView("history", "running")).toBe(false);
    expect(isHistoryTurnView("live", "succeeded")).toBe(false);
  });

  it("effective history: sender page still live thread but poll reached succeeded", () => {
    expect(isEffectiveHistoryTurnView("live", "succeeded")).toBe(true);
    expect(isEffectiveHistoryTurnView("live", "running")).toBe(false);
  });

  it("effective history: observer opens running session from list", () => {
    expect(isEffectiveHistoryTurnView("live", "running")).toBe(false);
    expect(isEffectiveHistoryTurnView("history", "running")).toBe(false);
  });
});
