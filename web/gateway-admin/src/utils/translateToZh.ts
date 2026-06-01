/** 浏览器侧整段翻译为简体中文；失败走网关 LLM。Author: kejiqing */

import { proxyHttp } from "../api/client";

const TRANSLATE_CHUNK = 3000;
const BROWSER_TRANSLATOR_TIMEOUT_MS = 4_000;
const GATEWAY_TRANSLATE_TIMEOUT_MS = 120_000;
const GATEWAY_CHUNK_DELAY_MS = 200;

function cjkRatio(text: string): number {
  const chars = text.replace(/\s/g, "");
  if (!chars.length) return 0;
  const cjk = chars.match(/[\u4e00-\u9fff\u3400-\u4dbf]/g);
  return (cjk?.length ?? 0) / chars.length;
}

/** 已主要为中文则跳过机器翻译。 */
export function mostlyChinese(text: string): boolean {
  return cjkRatio(text.trim()) >= 0.35;
}

function splitForTranslation(text: string, maxLen: number): string[] {
  const trimmed = text.trim();
  if (!trimmed) return [];
  if (trimmed.length <= maxLen) return [trimmed];

  const chunks: string[] = [];
  let rest = trimmed;
  while (rest.length > maxLen) {
    let cut = rest.lastIndexOf("\n\n", maxLen);
    if (cut < maxLen * 0.4) cut = rest.lastIndexOf("\n", maxLen);
    if (cut < maxLen * 0.4) cut = rest.lastIndexOf(" ", maxLen);
    if (cut < maxLen * 0.25) cut = maxLen;
    chunks.push(rest.slice(0, cut).trim());
    rest = rest.slice(cut).trim();
  }
  if (rest) chunks.push(rest);
  return chunks.filter(Boolean);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_, reject) => {
        timer = setTimeout(() => reject(new Error(`${label} 超时（${ms / 1000}s）`)), ms);
      }),
    ]);
  } finally {
    if (timer) clearTimeout(timer);
  }
}

async function translateWithBrowserApi(text: string): Promise<string | null> {
  const Tr = (globalThis as { Translator?: { create: (o: { sourceLanguage: string; targetLanguage: string }) => Promise<{ translate: (t: string) => Promise<string> }> } }).Translator;
  if (!Tr) return null;
  try {
    const translator = await withTimeout(
      Tr.create({ sourceLanguage: "en", targetLanguage: "zh" }),
      BROWSER_TRANSLATOR_TIMEOUT_MS,
      "浏览器翻译模型加载"
    );
    const out = await withTimeout(translator.translate(text), BROWSER_TRANSLATOR_TIMEOUT_MS, "浏览器翻译");
    return out?.trim() || null;
  } catch {
    return null;
  }
}

async function translateWithGateway(gatewayBase: string, text: string): Promise<string> {
  const res = await withTimeout(
    proxyHttp<{ translatedText?: string }>(gatewayBase, "POST", "/v1/gateway/translate", {
      text,
      targetLanguage: "zh-CN",
    }),
    GATEWAY_TRANSLATE_TIMEOUT_MS,
    "网关 LLM 翻译"
  );
  const out = res.translatedText?.trim();
  if (!out) throw new Error("网关翻译结果为空");
  return out;
}

async function translateChunk(text: string, gatewayBase: string): Promise<string> {
  const browser = await translateWithBrowserApi(text);
  if (browser) return browser;
  return translateWithGateway(gatewayBase, text);
}

/** 将非中文正文译为简体中文；已是中文则原样返回。 */
export async function translateTextToZh(
  text: string,
  gatewayBase: string,
  onUnitDone?: () => void
): Promise<string> {
  const src = text.trim();
  if (!src) return "";
  if (mostlyChinese(src)) {
    onUnitDone?.();
    return src;
  }

  const chunks = splitForTranslation(src, TRANSLATE_CHUNK);
  const out: string[] = [];
  for (let i = 0; i < chunks.length; i += 1) {
    const chunk = chunks[i];
    if (mostlyChinese(chunk)) {
      out.push(chunk);
    } else {
      if (i > 0) await sleep(GATEWAY_CHUNK_DELAY_MS);
      out.push(await translateChunk(chunk, gatewayBase));
    }
    onUnitDone?.();
  }
  return out.join("\n\n");
}

export function countTranslateUnits(turns: Array<{ userText: string; assistantText: string }>): number {
  let units = 0;
  for (const t of turns) {
    const user = t.userText.trim();
    if (user) {
      if (mostlyChinese(user)) units += 1;
      else units += splitForTranslation(user, TRANSLATE_CHUNK).length || 1;
    }
    const assistant = t.assistantText.trim();
    if (assistant) {
      if (mostlyChinese(assistant)) units += 1;
      else units += splitForTranslation(assistant, TRANSLATE_CHUNK).length || 1;
    }
  }
  return Math.max(units, 1);
}

export interface TranslateProgress {
  doneUnits: number;
  totalUnits: number;
  detail?: string;
}

export interface TranslatedTurn {
  index: number;
  turnId: string;
  userText: string;
  assistantText: string;
  userTextZh: string;
  assistantTextZh: string;
}

export async function translateConversationTurns(
  gatewayBase: string,
  turns: Array<{
    index: number;
    turnId: string;
    userText: string;
    assistantText: string;
  }>,
  onProgress?: (progress: TranslateProgress) => void
): Promise<TranslatedTurn[]> {
  const totalUnits = countTranslateUnits(turns);
  let doneUnits = 0;
  const bump = (detail: string) => {
    doneUnits += 1;
    onProgress?.({ doneUnits, totalUnits, detail });
  };

  const result: TranslatedTurn[] = [];
  for (const t of turns) {
    const userTextZh = await translateTextToZh(t.userText, gatewayBase, () =>
      bump(`轮次 ${t.index} · 用户`)
    );
    let assistantTextZh = "（无助手回复）";
    if (t.assistantText.trim()) {
      assistantTextZh = await translateTextToZh(t.assistantText, gatewayBase, () =>
        bump(`轮次 ${t.index} · 助手`)
      );
    } else {
      bump(`轮次 ${t.index} · 助手（空）`);
    }
    result.push({
      ...t,
      userTextZh,
      assistantTextZh,
    });
  }
  return result;
}

export function formatTranslatedConversation(turns: TranslatedTurn[]): string {
  return turns
    .map(
      (t) =>
        `## 轮次 ${t.index}\n\n**用户**\n\n${t.userTextZh}\n\n**助手**\n\n${t.assistantTextZh}`
    )
    .join("\n\n---\n\n");
}
