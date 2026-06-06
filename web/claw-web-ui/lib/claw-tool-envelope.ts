/** Tool result envelope (L2 v1.1) + ```claw-tool fence parsing. Author: kejiqing */

export type ClawToolPayloadKind =
  | "file_write"
  | "file_edit"
  | "file_read"
  | "bash"
  | "generic";

export type ClawStructuredPatchHunk = {
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: string[];
};

export type ClawFileWritePayload = {
  type: string;
  filePath: string;
  content?: string;
  structuredPatch?: ClawStructuredPatchHunk[];
  originalFile?: string | null;
  gitDiff?: unknown;
};

export type ClawToolEnvelope = {
  type: "tool.result";
  toolCallId: string;
  toolName: string;
  ok: boolean;
  summary: string;
  payloadKind: ClawToolPayloadKind;
  payload: unknown;
  error?: string;
};

const FENCE_RE = /```claw-tool\s*\n([\s\S]*?)```/g;

export function parseClawToolEnvelopes(content: string): ClawToolEnvelope[] {
  const out: ClawToolEnvelope[] = [];
  for (const m of content.matchAll(FENCE_RE)) {
    const raw = m[1]?.trim();
    if (!raw) continue;
    try {
      const v = JSON.parse(raw) as ClawToolEnvelope;
      if (v?.type === "tool.result" && v.toolCallId && v.payloadKind) {
        out.push(v);
      }
    } catch {
      /* skip malformed fence */
    }
  }
  return out;
}

export function stripClawToolFences(content: string): string {
  return content.replace(FENCE_RE, "").trim();
}

export function formatClawToolFence(envelope: ClawToolEnvelope): string {
  return `\n\n\`\`\`claw-tool\n${JSON.stringify(envelope)}\n\`\`\`\n\n`;
}
