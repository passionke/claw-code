/** Claw Web conversation model (PG-backed). Author: kejiqing */

/** One user send in a session (= gateway run / AG-UI runId scope). */
export type ClawTunnelRecord = {
  tunnelId: string;
  runId?: string | null;
  status: "pending" | "streaming" | "completed" | "failed";
  userPreview: string;
  errorPreview?: string | null;
  startedAtMs: number;
  finishedAtMs?: number | null;
};

/** One chat bubble (user or assistant) under a tunnel. */
export type ClawTunnelMessage = {
  tunnelId: string;
  role: "user" | "assistant";
  messageId: string;
  content: string;
  createdAtMs: number;
  runId?: string | null;
};

export type ClawSessionSummary = {
  sessionId: string;
  projectId: string;
  title: string;
  createdAtMs: number;
  updatedAtMs: number;
  /** Set when archived; omitted from default list. */
  archivedAtMs?: number | null;
};

export type ClawSessionRecord = ClawSessionSummary & {
  tunnels?: ClawTunnelRecord[];
  messages: ClawTunnelMessage[];
};

export { projectIdFromDsId } from "@/lib/claw-project-types";

export function deriveTitle(messages: ClawTunnelMessage[]): string {
  const firstUser = messages.find((m) => m.role === "user" && m.content.trim());
  if (!firstUser) return "新对话";
  const t = firstUser.content.trim().replace(/\s+/g, " ");
  return t.length > 36 ? `${t.slice(0, 33)}…` : t;
}
