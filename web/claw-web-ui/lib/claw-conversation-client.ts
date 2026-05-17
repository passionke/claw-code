/** Browser client → BFF (PostgreSQL). Author: kejiqing */

import type { ClawSessionRecord, ClawSessionSummary, ClawTunnelMessage } from "@/lib/claw-conversation-types";

async function parseJson<T>(res: Response): Promise<T> {
  const body = (await res.json()) as T & { error?: string };
  if (!res.ok) {
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
  return body;
}

export async function fetchConversationIndex(projectId: string): Promise<{
  activeSessionId: string | null;
  sessions: ClawSessionSummary[];
}> {
  const res = await fetch(`/api/claw/projects/${encodeURIComponent(projectId)}/conversations`, {
    cache: "no-store",
  });
  return parseJson(res);
}

export async function fetchSession(
  projectId: string,
  sessionId: string,
): Promise<ClawSessionRecord | null> {
  const res = await fetch(
    `/api/claw/projects/${encodeURIComponent(projectId)}/conversations/${encodeURIComponent(sessionId)}`,
    { cache: "no-store" },
  );
  if (res.status === 404) return null;
  return parseJson(res);
}

export async function createSessionApi(
  projectId: string,
  sessionId: string,
): Promise<ClawSessionRecord> {
  const res = await fetch(`/api/claw/projects/${encodeURIComponent(projectId)}/conversations`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ sessionId }),
  });
  return parseJson(res);
}

export async function saveSessionMessagesApi(
  projectId: string,
  sessionId: string,
  messages: ClawTunnelMessage[],
): Promise<ClawSessionRecord> {
  const res = await fetch(
    `/api/claw/projects/${encodeURIComponent(projectId)}/conversations/${encodeURIComponent(sessionId)}`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ messages }),
    },
  );
  return parseJson(res);
}

export async function setActiveSessionApi(
  projectId: string,
  sessionId: string,
): Promise<void> {
  const res = await fetch(
    `/api/claw/projects/${encodeURIComponent(projectId)}/conversations/active`,
    {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ sessionId }),
    },
  );
  await parseJson(res);
}

export async function migrateLocalToPg(
  projectId: string,
  payload: { activeSessionId: string | null; sessions: ClawSessionRecord[] },
): Promise<void> {
  const res = await fetch(
    `/api/claw/projects/${encodeURIComponent(projectId)}/conversations/migrate`,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    },
  );
  await parseJson(res);
}

export function notifyStoreUpdated(): void {
  window.dispatchEvent(new Event("claw-conversations-updated"));
}

export function subscribeStore(onChange: () => void): () => void {
  window.addEventListener("claw-conversations-updated", onChange);
  return () => window.removeEventListener("claw-conversations-updated", onChange);
}
