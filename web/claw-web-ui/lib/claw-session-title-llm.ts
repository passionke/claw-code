/** Server-only: LLM one-line session title (OpenAI-compatible API). Author: kejiqing */

import { deriveTitle } from "@/lib/claw-conversation-types";
import type { ClawTunnelMessage } from "@/lib/claw-conversation-types";

function transcriptSnippet(messages: ClawTunnelMessage[], maxLen = 1800): string {
  const lines: string[] = [];
  for (const m of messages) {
    const t = m.content.trim();
    if (!t) continue;
    const head = t.length > 400 ? `${t.slice(0, 397)}…` : t;
    lines.push(`${m.role === "user" ? "用户" : "助手"}: ${head}`);
    if (lines.join("\n").length >= maxLen) break;
  }
  return lines.join("\n").slice(0, maxLen);
}

async function llmTitle(transcript: string): Promise<string | null> {
  const base = process.env.CLAW_TITLE_LLM_BASE_URL?.trim();
  const key = process.env.CLAW_TITLE_LLM_API_KEY?.trim();
  const model = process.env.CLAW_TITLE_LLM_MODEL?.trim() || "deepseek-chat";
  if (!base || !key) return null;

  const url = `${base.replace(/\/$/, "")}/chat/completions`;
  const res = await fetch(url, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${key}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model,
      temperature: 0.2,
      max_tokens: 40,
      messages: [
        {
          role: "system",
          content:
            "根据对话摘录生成一个简短中文标题（6–14 字），概括主题。只输出标题本身，不要引号、标点或解释。",
        },
        { role: "user", content: transcript },
      ],
    }),
  });
  if (!res.ok) return null;
  const body = (await res.json()) as {
    choices?: { message?: { content?: string } }[];
  };
  const raw = body.choices?.[0]?.message?.content?.trim();
  if (!raw) return null;
  const oneLine = raw.replace(/\s+/g, " ").replace(/^["'「『]|["'」』]$/g, "");
  return oneLine.length > 48 ? `${oneLine.slice(0, 45)}…` : oneLine;
}

/** Prefer LLM title when configured; else first user line. Author: kejiqing */
export async function generateSessionTitle(messages: ClawTunnelMessage[]): Promise<string> {
  const hasAssistant = messages.some((m) => m.role === "assistant" && m.content.trim());
  if (!hasAssistant) return deriveTitle(messages);

  const transcript = transcriptSnippet(messages);
  if (!transcript) return deriveTitle(messages);

  try {
    const llm = await llmTitle(transcript);
    if (llm && llm.length > 0) return llm;
  } catch {
    /* fallback */
  }
  return deriveTitle(messages);
}
