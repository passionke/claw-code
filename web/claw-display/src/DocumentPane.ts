import { marked } from "marked";
import { enhanceAssistantProse, type WorkspaceMediaUrl } from "./renderMarkdown";
import type { CdpEvent } from "./types";

marked.setOptions({
  gfm: true,
  breaks: true,
});

interface TurnNote {
  kind: "tool" | "system" | "error";
  text: string;
  toolName?: string;
  toolSummary?: string;
}

interface TurnBlock {
  user: string;
  assistantMd: string;
  thinking: string[];
  notes: TurnNote[];
  streaming: boolean;
}

/** Multi-turn IM transcript: user prompts + assistant markdown + tool notes. Author: kejiqing */
export class DocumentPane {
  private readonly root: HTMLElement;
  private readonly list: HTMLElement;
  private readonly workspaceMediaUrl?: WorkspaceMediaUrl;
  private turns: TurnBlock[] = [];
  private activeIndex = -1;
  private proseGeneration = 0;

  constructor(root: HTMLElement, workspaceMediaUrl?: WorkspaceMediaUrl) {
    this.root = root;
    this.workspaceMediaUrl = workspaceMediaUrl;
    this.root.innerHTML = "";
    this.root.classList.add("claw-transcript-root");
    this.list = document.createElement("div");
    this.list.className = "claw-transcript";
    this.root.appendChild(this.list);
  }

  clear(): void {
    this.turns = [];
    this.activeIndex = -1;
    this.renderAll();
  }

  beginTurn(user = ""): void {
    this.sealActiveTurn();
    this.turns.push({
      user,
      assistantMd: "",
      thinking: [],
      notes: [],
      streaming: true,
    });
    this.activeIndex = this.turns.length - 1;
    this.renderAll();
    this.scrollToBottom();
  }

  handle(frame: CdpEvent): void {
    if (frame.ev === "turn.begin") {
      this.beginTurn(frame.user ?? "");
      return;
    }
    if (frame.ev === "tool.call") {
      this.ensureActiveTurn();
      const turn = this.turns[this.activeIndex];
      if (!turn) return;
      turn.notes.push({
        kind: "tool",
        text: "",
        toolName: frame.name,
        toolSummary: frame.summary,
      });
      this.renderAll();
      this.scrollToBottom();
      return;
    }
    if (frame.ev === "transcript.note") {
      this.ensureActiveTurn();
      const turn = this.turns[this.activeIndex];
      if (!turn) return;
      if (
        frame.kind === "system" &&
        /auto-compact/i.test(frame.text)
      ) {
        return;
      }
      turn.notes.push({ kind: frame.kind, text: frame.text });
      this.renderAll();
      this.scrollToBottom();
      return;
    }
    if (frame.ev === "content.delta" && frame.text) {
      this.ensureActiveTurn();
      const turn = this.turns[this.activeIndex];
      if (!turn) return;
      turn.assistantMd += frame.text;
      turn.streaming = true;
      this.renderAll();
      this.scrollToBottom();
      return;
    }
    if (frame.ev === "content.flush") {
      const turn = this.turns[this.activeIndex];
      if (turn) turn.streaming = false;
      this.renderAll();
      return;
    }
    if (frame.ev === "thinking") {
      // Web worker: thinking is status-bar only (Rust skips CDP); ignore if any leak.
      return;
    }
    if (frame.ev === "status" && (frame.phase === "done" || frame.phase === "failed")) {
      this.sealActiveTurn();
      this.renderAll();
      this.scrollToBottom();
    }
  }

  private ensureActiveTurn(): void {
    if (this.activeIndex >= 0 && this.turns[this.activeIndex]) return;
    this.beginTurn("");
  }

  private sealActiveTurn(): void {
    if (this.activeIndex < 0) return;
    const turn = this.turns[this.activeIndex];
    if (turn) turn.streaming = false;
  }

  private renderAll(): void {
    this.proseGeneration += 1;
    this.list.innerHTML = "";
    if (!this.turns.length) {
      const empty = document.createElement("div");
      empty.className = "claw-transcript-empty";
      empty.textContent = "连接后在此查看对话";
      this.list.appendChild(empty);
      return;
    }
    for (const turn of this.turns) {
      const el = document.createElement("section");
      el.className = "claw-turn";

      if (turn.user.trim()) {
        const user = document.createElement("div");
        user.className = "claw-turn-user";
        user.textContent = turn.user;
        el.appendChild(user);
      }

      for (const note of turn.thinking) {
        const thinking = document.createElement("div");
        thinking.className = "claw-thinking-note";
        thinking.textContent = note;
        el.appendChild(thinking);
      }

      const toolNotes = turn.notes.filter((n) => n.kind === "tool");
      const otherNotes = turn.notes.filter((n) => n.kind !== "tool");
      if (toolNotes.length) {
        el.appendChild(this.renderToolGroup(toolNotes));
      }
      for (const note of otherNotes) {
        el.appendChild(this.renderNote(note));
      }

      if (turn.assistantMd.trim() || turn.streaming) {
        const assistant = document.createElement("article");
        assistant.className = "claw-document-prose claw-turn-assistant";
        if (turn.streaming) assistant.classList.add("streaming");
        assistant.innerHTML = marked.parse(turn.assistantMd, { async: false }) as string;
        el.appendChild(assistant);
        const gen = this.proseGeneration;
        void enhanceAssistantProse(assistant, this.workspaceMediaUrl).then(() => {
          if (gen === this.proseGeneration) this.scrollToBottom();
        });
      }

      this.list.appendChild(el);
    }
  }

  private renderToolGroup(notes: TurnNote[]): HTMLElement {
    const details = document.createElement("details");
    details.className = "claw-tools-fold";
    const summary = document.createElement("summary");
    summary.textContent = `Tools (${notes.length})`;
    details.appendChild(summary);
    const list = document.createElement("div");
    list.className = "claw-tools-list";
    for (const note of notes) {
      list.appendChild(this.renderNote(note));
    }
    details.appendChild(list);
    return details;
  }

  private renderNote(note: TurnNote): HTMLElement {
    if (note.kind === "tool" && note.toolSummary) {
      const card = document.createElement("div");
      card.className = "claw-tool-card claw-tool-card-inline";
      const badge = document.createElement("span");
      badge.className = "claw-tool-badge";
      badge.textContent = note.toolName || "Tool";
      const cmd = document.createElement("span");
      cmd.className = "claw-tool-cmd";
      cmd.textContent = note.toolSummary;
      card.appendChild(badge);
      card.appendChild(cmd);
      return card;
    }
    if (note.kind === "tool") {
      const details = document.createElement("details");
      details.className = "claw-tool-card";
      const summary = document.createElement("summary");
      summary.className = "claw-tool-card-summary";
      const badge = document.createElement("span");
      badge.className = "claw-tool-badge";
      badge.textContent = note.toolName || "Tool";
      const cmd = document.createElement("span");
      cmd.className = "claw-tool-cmd";
      cmd.textContent = note.toolSummary || note.text || "";
      summary.appendChild(badge);
      summary.appendChild(cmd);
      details.appendChild(summary);
      if (note.text.trim() && !note.toolSummary) {
        const body = document.createElement("pre");
        body.className = "claw-tool-card-body";
        body.textContent = note.text;
        details.appendChild(body);
      }
      return details;
    }
    const div = document.createElement("div");
    div.className =
      note.kind === "error" ? "claw-system-note claw-system-error" : "claw-system-note";
    div.textContent = note.text;
    return div;
  }

  private scrollToBottom(): void {
    this.root.scrollTop = this.root.scrollHeight;
  }
}
