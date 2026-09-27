/** Default collapsed/expanded map for POST /v1/responses nerogate.display. Author: kejiqing */

export const RESPONSES_DISPLAY_KINDS = [
  "thinking",
  "search",
  "read",
  "edit",
  "shell",
  "mcp",
  "ask",
] as const;

export type ResponsesDisplayKind = (typeof RESPONSES_DISPLAY_KINDS)[number];
export type ResponsesDisplayMap = Record<string, "collapsed" | "expanded">;

const STORAGE_KEY = "claw.playground.responses-display";
const DEFAULT_EXPANDED = new Set<string>(["shell", "ask"]);

export function defaultResponsesDisplay(): ResponsesDisplayMap {
  const out: ResponsesDisplayMap = {};
  for (const kind of RESPONSES_DISPLAY_KINDS) {
    out[kind] = DEFAULT_EXPANDED.has(kind) ? "expanded" : "collapsed";
  }
  return out;
}

export function loadResponsesDisplay(): ResponsesDisplayMap {
  const base = defaultResponsesDisplay();
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return base;
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    if (!parsed || typeof parsed !== "object") return base;
    for (const kind of RESPONSES_DISPLAY_KINDS) {
      const mode = parsed[kind];
      if (mode === "collapsed" || mode === "expanded") base[kind] = mode;
    }
  } catch {
    /* ignore */
  }
  return base;
}

export function saveResponsesDisplay(map: ResponsesDisplayMap): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
  } catch {
    /* ignore */
  }
}
