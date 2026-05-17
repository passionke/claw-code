/** One-time read of legacy localStorage index for PG migrate. Author: kejiqing */

import { STORAGE_THREAD_ID } from "@/lib/claw-config";
import type { ClawSessionRecord, ClawTunnelMessage } from "@/lib/claw-conversation-types";
import { deriveTitle } from "@/lib/claw-conversation-types";

const STORAGE_CONVERSATIONS_V1 = "claw_web_conversations_v1";

type LegacyRoot = {
  version: 1;
  projects: Record<
    string,
    {
      activeSessionId: string | null;
      sessions: ClawSessionRecord[];
    }
  >;
};

export function readLegacyProject(projectId: string): {
  activeSessionId: string | null;
  sessions: ClawSessionRecord[];
} | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = localStorage.getItem(STORAGE_CONVERSATIONS_V1);
    if (!raw) return legacySingleThread(projectId);
    const parsed = JSON.parse(raw) as LegacyRoot;
    const proj = parsed.projects?.[projectId];
    if (!proj) return legacySingleThread(projectId);
    return { activeSessionId: proj.activeSessionId, sessions: proj.sessions };
  } catch {
    return null;
  }
}

function legacySingleThread(projectId: string): {
  activeSessionId: string | null;
  sessions: ClawSessionRecord[];
} | null {
  const legacy = localStorage.getItem(STORAGE_THREAD_ID)?.trim();
  if (!legacy) return null;
  const now = Date.now();
  return {
    activeSessionId: legacy,
    sessions: [
      {
        projectId,
        sessionId: legacy,
        title: "导入的对话",
        createdAtMs: now,
        updatedAtMs: now,
        messages: [] as ClawTunnelMessage[],
      },
    ],
  };
}

export function clearLegacyStorage(): void {
  localStorage.removeItem(STORAGE_CONVERSATIONS_V1);
  localStorage.removeItem(STORAGE_THREAD_ID);
}

export function normalizeSessions(
  projectId: string,
  sessions: ClawSessionRecord[],
): ClawSessionRecord[] {
  return sessions.map((s) => ({
    ...s,
    projectId,
    title: s.title || deriveTitle(s.messages ?? []),
    messages: s.messages ?? [],
  }));
}
