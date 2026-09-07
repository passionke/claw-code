import { describe, expect, it } from "vitest";
import { isPlanConfirmUserPrompt, planPhaseFromPlanStatus } from "./planConfirm";

describe("isPlanConfirmUserPrompt", () => {
  it("accepts bare confirms", () => {
    expect(isPlanConfirmUserPrompt("确认")).toBe(true);
    expect(isPlanConfirmUserPrompt(" 确认执行 ")).toBe(true);
    expect(isPlanConfirmUserPrompt("confirm!")).toBe(true);
    expect(isPlanConfirmUserPrompt("OK")).toBe(true);
  });

  it("rejects substantive prompts", () => {
    expect(isPlanConfirmUserPrompt("确认品牌改成 Aurora")).toBe(false);
    expect(isPlanConfirmUserPrompt("帮我做一个网页")).toBe(false);
    expect(isPlanConfirmUserPrompt("")).toBe(false);
  });
});

describe("planPhaseFromPlanStatus", () => {
  it("maps awaiting_confirm", () => {
    expect(planPhaseFromPlanStatus("awaiting_confirm")).toBe("awaiting_confirm");
    expect(planPhaseFromPlanStatus("sealed")).toBe("confirmed");
  });
});
