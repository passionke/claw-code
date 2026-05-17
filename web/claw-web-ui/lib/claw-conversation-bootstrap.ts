/** Boot: ensure user cookie, migrate localStorage → PG, ensure active session. Author: kejiqing */

import type { ClawWebUser } from "@/lib/claw-user-types";
import {
  clearLegacyStorage,
  normalizeSessions,
  readLegacyProject,
} from "@/lib/claw-conversation-local-backup";
import {
  createSessionApi,
  fetchConversationIndex,
  migrateLocalToPg,
} from "@/lib/claw-conversation-client";
import { projectIdFromDsId } from "@/lib/claw-conversation-types";

export function randomSessionId(): string {
  if (typeof crypto !== "undefined" && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  return `session-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

async function ensureWebUser(): Promise<ClawWebUser> {
  const res = await fetch("/api/claw/me", { cache: "no-store" });
  if (!res.ok) {
    const body = (await res.json()) as { error?: string };
    throw new Error(body.error ?? `GET /api/claw/me HTTP ${res.status}`);
  }
  return (await res.json()) as ClawWebUser;
}

async function ensureWebProject(dsId: number): Promise<void> {
  const res = await fetch("/api/claw/projects", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ dsId }),
  });
  if (!res.ok) {
    const body = (await res.json()) as { error?: string };
    throw new Error(body.error ?? `POST /api/claw/projects HTTP ${res.status}`);
  }
}

export async function bootstrapProject(dsId: number): Promise<string> {
  await ensureWebUser();
  await ensureWebProject(dsId);
  const projectId = projectIdFromDsId(dsId);
  const legacy = readLegacyProject(projectId);
  if (legacy && legacy.sessions.length > 0) {
    await migrateLocalToPg(projectId, {
      activeSessionId: legacy.activeSessionId,
      sessions: normalizeSessions(projectId, legacy.sessions),
    });
    clearLegacyStorage();
  }
  const index = await fetchConversationIndex(projectId);
  const active = index.activeSessionId;
  if (active && index.sessions.some((s) => s.sessionId === active)) {
    return active;
  }
  if (index.sessions.length > 0) {
    const newest = index.sessions[0].sessionId;
    return newest;
  }
  const created = await createSessionApi(projectId, randomSessionId());
  return created.sessionId;
}
