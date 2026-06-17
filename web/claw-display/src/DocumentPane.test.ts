import { describe, expect, it } from "vitest";
import { DocumentPane } from "./DocumentPane";

describe("DocumentPane transcript", () => {
  it("renders MOSS banner with brand colors", () => {
    const root = document.createElement("div");
    const pane = new DocumentPane(root);
    pane.handle({
      ev: "banner.moss",
      mossArt: [" ███╗   ███╗ ██████╗ ███████╗ ███████╗"],
      w550Art: [" ██████╗     ██████╗      ██████╗      ██╗    ██╗"],
      tagline: "让人类保持理智，是一种奢求",
      connected: "Connected: claude-sonnet-4-6 via Anthropic",
      meta: [{ key: "Model", value: "claude-sonnet-4-6" }],
      hint: "Type /help for commands",
    });
    const banner = root.querySelector(".moss-banner");
    expect(banner).not.toBeNull();
    expect(root.querySelector(".moss-ascii-moss")?.textContent).toContain("███╗");
    expect(root.querySelector(".moss-ascii-550w")?.textContent).toContain("██████╗");
    expect(root.querySelector(".moss-eye")?.textContent).toBe("●");
    expect(root.textContent).toContain("让人类保持理智");
  });

  it("keeps multiple turns", () => {
    const root = document.createElement("div");
    const pane = new DocumentPane(root);
    pane.handle({ ev: "turn.begin", user: "第一句" });
    pane.handle({ ev: "content.delta", mime: "text/markdown", text: "回复一\n" });
    pane.handle({ ev: "content.flush" });
    pane.handle({ ev: "turn.begin", user: "第二句" });
    pane.handle({ ev: "content.delta", mime: "text/markdown", text: "回复二\n" });
    const users = root.querySelectorAll(".claw-turn-user");
    expect(users.length).toBe(2);
    expect(users[0].textContent).toBe("第一句");
    expect(users[1].textContent).toBe("第二句");
    expect(root.textContent).toContain("回复一");
    expect(root.textContent).toContain("回复二");
  });
});
