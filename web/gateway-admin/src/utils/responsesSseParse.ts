/** Parse POST /v1/responses stream=true into one interleaved block list. Author: kejiqing */

export type ResponsesDisplayMode = "collapsed" | "expanded";

export type ResponsesTagStatus = "running" | "done" | "failed";

export type ResponsesTextBlock = {
  id: string;
  type: "text";
  text: string;
};

export type ResponsesTagBlock = {
  id: string;
  type: "tag";
  kind: string;
  title: string;
  body: string;
  display: ResponsesDisplayMode;
  status: ResponsesTagStatus;
};

export type ResponsesAskBlock = {
  id: string;
  type: "ask";
  questionId: string;
  question: string;
  options: string[];
  display: ResponsesDisplayMode;
};

export type ResponsesStreamBlock =
  | ResponsesTextBlock
  | ResponsesTagBlock
  | ResponsesAskBlock;

export type ResponsesStreamState = {
  blocks: ResponsesStreamBlock[];
  turnId: string;
  sessionId: string;
  completed: boolean;
  error: string;
};

export function emptyResponsesStreamState(): ResponsesStreamState {
  return {
    blocks: [],
    turnId: "",
    sessionId: "",
    completed: false,
    error: "",
  };
}

export function splitSseFrames(buffer: string): {
  frames: { event: string; data: string }[];
  rest: string;
} {
  const parts = buffer.split("\n\n");
  const rest = parts.pop() ?? "";
  const frames: { event: string; data: string }[] = [];
  for (const part of parts) {
    let event = "message";
    const dataLines: string[] = [];
    for (const rawLine of part.split("\n")) {
      const line = rawLine.replace(/\r$/, "");
      if (line.startsWith("event:")) event = line.slice(6).trim();
      else if (line.startsWith("data:")) dataLines.push(line.slice(5).trim());
    }
    const data = dataLines.join("\n");
    if (!data || data === "[DONE]") continue;
    frames.push({ event, data });
  }
  return { frames, rest };
}

function asRecord(raw: unknown): Record<string, unknown> | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return null;
  return raw as Record<string, unknown>;
}

function strField(data: Record<string, unknown>, key: string): string {
  const v = data[key];
  return typeof v === "string" ? v : "";
}

function nerogateObj(data: Record<string, unknown>): Record<string, unknown> | null {
  return asRecord(data.nerogate);
}

function nerogateKind(data: Record<string, unknown>, fallback: string): string {
  const kind = nerogateObj(data)?.kind;
  return typeof kind === "string" && kind.trim() ? kind : fallback;
}

function nerogateDisplay(
  data: Record<string, unknown>,
  fallback: ResponsesDisplayMode
): ResponsesDisplayMode {
  const display = nerogateObj(data)?.display;
  if (display === "collapsed" || display === "expanded") return display;
  return fallback;
}

function itemId(data: Record<string, unknown>): string {
  const direct = strField(data, "item_id");
  if (direct) return direct;
  const item = asRecord(data.item);
  if (!item) return "";
  return typeof item.id === "string" ? item.id : "";
}

function itemName(data: Record<string, unknown>): string {
  const item = asRecord(data.item);
  if (!item) return "";
  return typeof item.name === "string" ? item.name : "";
}

function itemArguments(data: Record<string, unknown>): string {
  const item = asRecord(data.item);
  if (!item) return "";
  return typeof item.arguments === "string" ? item.arguments : "";
}

function upsertTag(
  blocks: ResponsesStreamBlock[],
  next: Omit<ResponsesTagBlock, "type">
): ResponsesStreamBlock[] {
  const idx = blocks.findIndex((b) => b.type === "tag" && b.id === next.id);
  if (idx < 0) {
    return [...blocks, { type: "tag", ...next }];
  }
  const cur = blocks[idx];
  if (cur.type !== "tag") return blocks;
  const copy = blocks.slice();
  copy[idx] = {
    ...cur,
    kind: next.kind || cur.kind,
    title: next.title || cur.title,
    body: next.body || cur.body,
    display: next.display,
    status: next.status,
  };
  return copy;
}

function appendTagBody(
  blocks: ResponsesStreamBlock[],
  id: string,
  delta: string,
  kind: string,
  display: ResponsesDisplayMode
): ResponsesStreamBlock[] {
  const idx = blocks.findIndex((b) => b.type === "tag" && b.id === id);
  if (idx < 0) {
    return [
      ...blocks,
      {
        type: "tag",
        id,
        kind,
        title: kind,
        body: delta,
        display,
        status: "running",
      },
    ];
  }
  const cur = blocks[idx];
  if (cur.type !== "tag") return blocks;
  const copy = blocks.slice();
  copy[idx] = { ...cur, body: cur.body + delta };
  return copy;
}

function markTagDone(
  blocks: ResponsesStreamBlock[],
  id: string,
  status: ResponsesTagStatus
): ResponsesStreamBlock[] {
  return blocks.map((b) =>
    b.type === "tag" && b.id === id ? { ...b, status } : b
  );
}

function appendText(blocks: ResponsesStreamBlock[], delta: string): ResponsesStreamBlock[] {
  if (!delta) return blocks;
  const last = blocks[blocks.length - 1];
  if (last && last.type === "text") {
    const copy = blocks.slice();
    copy[copy.length - 1] = { ...last, text: last.text + delta };
    return copy;
  }
  return [
    ...blocks,
    { id: `text-${blocks.length}`, type: "text", text: delta },
  ];
}

/** Fold one Responses SSE event into the live timeline. Author: kejiqing */
export function applyResponsesSseEvent(
  state: ResponsesStreamState,
  event: string,
  raw: unknown
): ResponsesStreamState {
  const data = asRecord(raw);
  if (!data) return state;

  if (event === "response.created") {
    const response = asRecord(data.response);
    const turnId =
      (response && typeof response.id === "string" && response.id) ||
      strField(data, "id") ||
      state.turnId;
    return { ...state, turnId };
  }

  if (event === "response.output_text.delta") {
    return { ...state, blocks: appendText(state.blocks, strField(data, "delta")) };
  }

  if (event === "response.output_item.added") {
    const item = asRecord(data.item);
    const type = item && typeof item.type === "string" ? item.type : "";
    if (type === "reasoning") {
      const id = itemId(data) || "thinking";
      return {
        ...state,
        blocks: upsertTag(state.blocks, {
          id,
          kind: "thinking",
          title: "thinking",
          body: "",
          display: nerogateDisplay(data, "collapsed"),
          status: "running",
        }),
      };
    }
    if (type === "function_call") {
      const id = itemId(data);
      if (!id) return state;
      const name = itemName(data) || "tool";
      const args = itemArguments(data);
      return {
        ...state,
        blocks: upsertTag(state.blocks, {
          id,
          kind: "tool",
          title: args ? `${name}: ${args}` : name,
          body: "",
          display: "collapsed",
          status: "running",
        }),
      };
    }
    return state;
  }

  if (event === "response.reasoning_text.delta") {
    const id = itemId(data) || "thinking";
    return {
      ...state,
      blocks: appendTagBody(
        upsertTag(state.blocks, {
          id,
          kind: "thinking",
          title: "thinking",
          body: "",
          display: nerogateDisplay(data, "collapsed"),
          status: "running",
        }),
        id,
        strField(data, "delta"),
        "thinking",
        nerogateDisplay(data, "collapsed")
      ),
    };
  }

  if (
    event === "response.function_call_arguments.delta" ||
    event === "response.mcp_call_arguments.delta" ||
    event === "response.mcp_call.in_progress"
  ) {
    const id = itemId(data);
    if (!id) return state;
    const kind = nerogateKind(data, event.includes("mcp") ? "mcp" : "tool");
    const delta = strField(data, "delta");
      return {
        ...state,
        blocks: upsertTag(state.blocks, {
          id,
          kind,
          title: delta ? `${kind}: ${delta}` : kind,
          body: "",
          display: nerogateDisplay(data, kind === "shell" ? "expanded" : "collapsed"),
          status: "running",
        }),
      };
  }

  if (event === "response.nerogate.shell_output.delta") {
    const id = itemId(data);
    if (!id) return state;
    return {
      ...state,
      blocks: appendTagBody(
        state.blocks,
        id,
        strField(data, "delta"),
        "shell",
        "expanded"
      ),
    };
  }

  if (event === "response.function_call_arguments.done") {
    const id = itemId(data);
    if (!id) return state;
    return { ...state, blocks: markTagDone(state.blocks, id, "done") };
  }

  if (event === "response.mcp_call.completed") {
    const id = itemId(data);
    if (!id) return state;
    return { ...state, blocks: markTagDone(state.blocks, id, "done") };
  }

  if (event === "response.mcp_call.failed") {
    const id = itemId(data);
    if (!id) return state;
    return { ...state, blocks: markTagDone(state.blocks, id, "failed") };
  }

  if (event === "response.nerogate.ask") {
    const questionId = strField(data, "questionId") || "q";
    const optionsRaw = data.options;
    const options = Array.isArray(optionsRaw)
      ? optionsRaw.filter((x): x is string => typeof x === "string")
      : [];
    const id = `ask-${questionId}`;
    const block: ResponsesAskBlock = {
      id,
      type: "ask",
      questionId,
      question: strField(data, "question"),
      options,
      display: nerogateDisplay(data, "expanded"),
    };
    const idx = state.blocks.findIndex((b) => b.type === "ask" && b.id === id);
    if (idx < 0) {
      return { ...state, blocks: [...state.blocks, block] };
    }
    const copy = state.blocks.slice();
    copy[idx] = block;
    return { ...state, blocks: copy };
  }

  if (event === "response.completed") {
    const response = asRecord(data.response);
    const turnId =
      (response && typeof response.id === "string" && response.id) || state.turnId;
    const sessionId =
      (response && asRecord(response.nerogate)?.sessionId) || state.sessionId;
    return {
      ...state,
      turnId: typeof turnId === "string" ? turnId : state.turnId,
      sessionId: typeof sessionId === "string" ? sessionId : state.sessionId,
      completed: true,
    };
  }

  return state;
}

export function reportTextFromBlocks(blocks: ResponsesStreamBlock[]): string {
  return blocks
    .filter((b): b is ResponsesTextBlock => b.type === "text")
    .map((b) => b.text)
    .join("");
}

export async function consumeResponsesSse(
  resp: Response,
  onState: (state: ResponsesStreamState) => void,
  initial: ResponsesStreamState = emptyResponsesStreamState(),
  signal?: AbortSignal
): Promise<ResponsesStreamState> {
  if (!resp.body) {
    const next = { ...initial, error: "响应没有正文", completed: true };
    onState(next);
    return next;
  }
  const sessionId = resp.headers.get("x-nerogate-session-id")?.trim() || initial.sessionId;
  let state: ResponsesStreamState = { ...initial, sessionId };
  onState(state);
  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  try {
    while (true) {
      if (signal?.aborted) break;
      const step = await reader.read();
      if (step.done) break;
      buf += decoder.decode(step.value, { stream: true });
      const { frames, rest } = splitSseFrames(buf);
      buf = rest;
      if (!frames.length) continue;
      for (const frame of frames) {
        let payload: unknown = frame.data;
        try {
          payload = JSON.parse(frame.data);
        } catch {
          continue;
        }
        state = applyResponsesSseEvent(state, frame.event, payload);
      }
      onState(state);
    }
    const tail = splitSseFrames(buf + "\n\n");
    for (const frame of tail.frames) {
      try {
        state = applyResponsesSseEvent(state, frame.event, JSON.parse(frame.data));
      } catch {
        /* ignore */
      }
    }
    state = { ...state, completed: true };
    onState(state);
    return state;
  } finally {
    try {
      await reader.cancel();
    } catch {
      /* ignore */
    }
  }
}
