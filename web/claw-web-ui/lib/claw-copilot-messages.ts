/** Map CopilotKit messages ↔ Claw tunnel storage. Author: kejiqing */

import type { ClawTunnelMessage } from "@/lib/claw-conversation-store";

type LooseMessage = Record<string, unknown>;

export function messageId(m: unknown): string {
  if (!m || typeof m !== "object") return `msg-${Date.now()}`;
  const id = (m as LooseMessage).id;
  return typeof id === "string" && id ? id : `msg-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

export function messageRole(m: unknown): "user" | "assistant" | null {
  if (!m || typeof m !== "object") return null;
  const role = (m as LooseMessage).role;
  if (role === "user" || role === "assistant") return role;
  return null;
}

export function messageContent(m: unknown): string {
  if (!m || typeof m !== "object") return "";
  const o = m as LooseMessage;
  if (typeof o.content === "string") return o.content;
  if (Array.isArray(o.content)) {
    return o.content
      .map((part) => {
        if (typeof part === "string") return part;
        if (part && typeof part === "object" && typeof (part as LooseMessage).text === "string") {
          return (part as LooseMessage).text as string;
        }
        return "";
      })
      .join("");
  }
  return "";
}

export function copilotMessagesToStored(messages: unknown[]): ClawTunnelMessage[] {
  const out: ClawTunnelMessage[] = [];
  let tunnelId = "";
  const now = Date.now();

  for (const m of messages) {
    const role = messageRole(m);
    const content = messageContent(m).trim();
    if (!role || !content) continue;
    if (role === "user") {
      tunnelId =
        typeof crypto !== "undefined" && crypto.randomUUID
          ? crypto.randomUUID()
          : `tunnel-${now}-${out.length}`;
    } else if (!tunnelId) {
      tunnelId = `tunnel-${now}-${out.length}`;
    }
    out.push({
      tunnelId,
      role,
      messageId: messageId(m),
      content,
      createdAtMs: now,
    });
  }
  return out;
}

export function storedToCopilotMessages(stored: ClawTunnelMessage[]): unknown[] {
  return stored.map((m) => ({
    id: m.messageId,
    role: m.role,
    content: m.content,
  }));
}
