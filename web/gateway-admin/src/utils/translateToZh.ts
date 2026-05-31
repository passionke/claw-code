/** 浏览器侧整段翻译为简体中文（无后端）。Author: kejiqing */

const MYMEMORY_CHUNK = 450;
const MYMEMORY_URL = "https://api.mymemory.translated.net/get";

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

async function translateWithBrowserApi(text: string): Promise<string | null> {
  const Tr = (globalThis as { Translator?: { create: (o: { sourceLanguage: string; targetLanguage: string }) => Promise<{ translate: (t: string) => Promise<string> }> } }).Translator;
  if (!Tr) return null;
  try {
    const translator = await Tr.create({ sourceLanguage: "en", targetLanguage: "zh" });
    return await translator.translate(text);
  } catch {
    try {
      const translator = await Tr.create({ sourceLanguage: "es", targetLanguage: "zh" });
      return await translator.translate(text);
    } catch {
      return null;
    }
  }
}

async function translateWithMyMemory(text: string): Promise<string> {
  const q = encodeURIComponent(text);
  const res = await fetch(`${MYMEMORY_URL}?q=${q}&langpair=en|zh-CN`, {
    method: "GET",
  });
  if (!res.ok) throw new Error(`翻译服务 HTTP ${res.status}`);
  const json = (await res.json()) as {
    responseStatus?: number;
    responseDetails?: string;
    responseData?: { translatedText?: string };
  };
  if (json.responseStatus && json.responseStatus !== 200) {
    throw new Error(json.responseDetails || "翻译服务返回错误");
  }
  const out = json.responseData?.translatedText?.trim();
  if (!out) throw new Error("翻译结果为空");
  return out;
}

async function translateChunk(text: string): Promise<string> {
  const browser = await translateWithBrowserApi(text);
  if (browser?.trim()) return browser.trim();
  return translateWithMyMemory(text);
}

/** 将非中文正文译为简体中文；已是中文则原样返回。 */
export async function translateTextToZh(text: string): Promise<string> {
  const src = text.trim();
  if (!src) return "";
  if (mostlyChinese(src)) return src;

  const chunks = splitForTranslation(src, MYMEMORY_CHUNK);
  const out: string[] = [];
  for (const chunk of chunks) {
    if (mostlyChinese(chunk)) {
      out.push(chunk);
    } else {
      out.push(await translateChunk(chunk));
    }
  }
  return out.join("\n\n");
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
  turns: Array<{
    index: number;
    turnId: string;
    userText: string;
    assistantText: string;
  }>,
  onProgress?: (done: number, total: number) => void
): Promise<TranslatedTurn[]> {
  const total = turns.length * 2;
  let done = 0;
  const bump = () => {
    done += 1;
    onProgress?.(done, total);
  };

  const result: TranslatedTurn[] = [];
  for (const t of turns) {
    const userTextZh = await translateTextToZh(t.userText);
    bump();
    const assistantTextZh = t.assistantText.trim()
      ? await translateTextToZh(t.assistantText)
      : "（无助手回复）";
    bump();
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
