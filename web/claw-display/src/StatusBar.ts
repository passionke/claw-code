import type { CdpEvent } from "./types";

/** Non-ANSI turn status (thinking / done / failed). Author: kejiqing */
export class StatusBar {
  private readonly el: HTMLElement;

  constructor(root: HTMLElement) {
    this.el = root;
    this.el.classList.add("claw-status-bar");
    this.el.textContent = "";
  }

  handle(frame: CdpEvent): void {
    if (frame.ev !== "status") return;
    this.el.classList.remove("thinking", "done", "failed");
    this.el.classList.add(frame.phase);
    const prefix =
      frame.phase === "done" ? "✔ " : frame.phase === "failed" ? "✘ " : "⠋ ";
    this.el.textContent = `${prefix}${frame.label}`;
  }

  reset(): void {
    this.el.classList.remove("thinking", "done", "failed");
    this.el.textContent = "";
  }
}
