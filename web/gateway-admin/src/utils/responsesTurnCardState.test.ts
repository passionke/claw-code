import { describe, expect, it } from "vitest";

import {
  deriveResponsesCardStatus,
  nextResponsesFinishedAtMs,
  resolveResponsesCardTimestamps,
  resolveTurnCardWallMs,
  shouldAdoptSessionTaskForResponsesCard,
  shouldDropStaleResponsesTaskTimes,
} from "./responsesTurnCardState";

describe("deriveResponsesCardStatus — SSE owns live card status", () => {
  it("maps in-flight stream to running (not stuck queued)", () => {
    expect(deriveResponsesCardStatus({ completed: false })).toBe("running");
  });

  it("maps completed stream to succeeded", () => {
    expect(deriveResponsesCardStatus({ completed: true })).toBe("succeeded");
  });

  it("maps stream error to failed even if completed", () => {
    expect(
      deriveResponsesCardStatus({ completed: true, error: "boom" })
    ).toBe("failed");
  });
});

describe("shouldAdoptSessionTaskForResponsesCard — do not take previous turn", () => {
  it("rejects poll before card turnId arrives", () => {
    expect(shouldAdoptSessionTaskForResponsesCard("", "T_prev")).toBe(false);
  });

  it("rejects poll when session latest turn is a different id", () => {
    expect(shouldAdoptSessionTaskForResponsesCard("T_new", "T_prev")).toBe(false);
  });

  it("adopts poll when turn ids match", () => {
    expect(shouldAdoptSessionTaskForResponsesCard("T_new", "T_new")).toBe(true);
  });

  it("adopts poll when server omits turnId but card already has one", () => {
    expect(shouldAdoptSessionTaskForResponsesCard("T_new", null)).toBe(true);
  });
});

describe("same-session next turn regression — queued + previous duration", () => {
  const prevCreated = 1_000_000;
  const prevFinished = 1_007_200; // 7.2s previous turn

  it("drops previous finishedAt when status forced back toward live", () => {
    expect(
      shouldDropStaleResponsesTaskTimes({
        status: "queued",
        cardTurnId: "T_new",
        taskTurnId: "T_prev",
        finishedAtMs: prevFinished,
      })
    ).toBe(true);

    expect(
      shouldDropStaleResponsesTaskTimes({
        status: "running",
        cardTurnId: "T_new",
        taskTurnId: "T_new",
        finishedAtMs: prevFinished,
      })
    ).toBe(true);
  });

  it("keeps times once this turn is terminal with matching turnId", () => {
    expect(
      shouldDropStaleResponsesTaskTimes({
        status: "succeeded",
        cardTurnId: "T_new",
        taskTurnId: "T_new",
        finishedAtMs: prevFinished,
      })
    ).toBe(false);
  });

  it("does not show previous-turn wall duration while card is queued/running", () => {
    const times = resolveResponsesCardTimestamps({
      responsesLive: true,
      cardTurnId: "T_new",
      taskTurnId: "T_prev",
      taskCreatedAtMs: prevCreated,
      taskFinishedAtMs: prevFinished,
      propCreatedAtMs: 2_000_000,
      propFinishedAtMs: undefined,
    });
    expect(times.createdAtMs).toBe(2_000_000);
    expect(times.finishedAtMs).toBeUndefined();
    expect(
      resolveTurnCardWallMs({
        status: "queued",
        createdAtMs: times.createdAtMs,
        finishedAtMs: times.finishedAtMs ?? prevFinished,
      })
    ).toBeNull();
    expect(
      resolveTurnCardWallMs({
        status: "running",
        createdAtMs: times.createdAtMs,
        finishedAtMs: prevFinished,
      })
    ).toBeNull();
  });

  it("shows duration only after this card reaches terminal with its own times", () => {
    const cardCreated = 2_000_000;
    const cardFinished = 2_003_500;
    expect(
      resolveTurnCardWallMs({
        status: "succeeded",
        createdAtMs: cardCreated,
        finishedAtMs: cardFinished,
      })
    ).toBe(3500);
  });
});

describe("nextResponsesFinishedAtMs", () => {
  it("clears finishedAt while running", () => {
    expect(nextResponsesFinishedAtMs("running", 99, 100)).toBeUndefined();
  });

  it("sets finishedAt on terminal, preserving an existing value", () => {
    expect(nextResponsesFinishedAtMs("succeeded", 50, 100)).toBe(50);
    expect(nextResponsesFinishedAtMs("failed", undefined, 100)).toBe(100);
  });
});
