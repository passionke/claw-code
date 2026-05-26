/** 从 solve 持久化的 JSON 字符串中提取 `message` 正文。Author: kejiqing */
export function extractSolveReportMessage(raw: string): string {
  const t = raw.trim();
  if (!t) return "";
  if (t.startsWith("{")) {
    try {
      const j = JSON.parse(t) as { message?: unknown };
      if (typeof j.message === "string" && j.message.trim()) {
        return j.message.trim();
      }
    } catch {
      /* not JSON */
    }
  }
  return t;
}
