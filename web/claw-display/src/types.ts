/** Claw Display Protocol (CDP) frame shapes. Author: kejiqing */

export interface MossBannerMetaRow {
  key: string;
  value: string;
}

export type CdpEvent =
  | { ev: "turn.begin"; user?: string }
  | { ev: "content.delta"; mime: string; text: string }
  | { ev: "content.flush" }
  | { ev: "status"; phase: "thinking" | "done" | "failed"; label: string }
  | { ev: "thinking"; chars: number | null; hidden: boolean }
  | { ev: "transcript.note"; kind: "tool" | "system" | "error"; text: string }
  | { ev: "tool.call"; name: string; summary: string }
  | {
      ev: "banner.moss";
      mossArt: string[];
      w550Art: string[];
      tagline: string;
      connected: string;
      meta: MossBannerMetaRow[];
      hint: string;
    };

export interface RouteResult {
  terminalText: string;
  frames: CdpEvent[];
}
