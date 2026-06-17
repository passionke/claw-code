/** Claw Display Protocol (CDP) frame shapes. Author: kejiqing */

export type CdpEvent =
  | { ev: "turn.begin"; user?: string }
  | { ev: "content.delta"; mime: string; text: string }
  | { ev: "content.flush" }
  | { ev: "status"; phase: "thinking" | "done" | "failed"; label: string }
  | { ev: "thinking"; chars: number | null; hidden: boolean }
  | { ev: "transcript.note"; kind: "tool" | "system" | "error"; text: string }
  | { ev: "tool.call"; name: string; summary: string };

export interface RouteResult {
  terminalText: string;
  frames: CdpEvent[];
}
