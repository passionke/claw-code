/** Merge gateway execution + tap events into one sorted timeline. Author: kejiqing */

export type TimelineEntry = {
  id: string;
  tsMs: number;
  tags: string[];
  message: string;
  isError: boolean;
};

export type TaskProgressRow = {
  updatedAtMs: number;
  currentTaskDesc: string;
  phase?: string;
};

export type TapEventRow = Record<string, unknown> & { type?: string };

export type TraceRow = Record<string, unknown>;

export type TaskSnapshot = {
  status: string;
  createdAtMs: number;
  startedAtMs?: number | null;
  finishedAtMs?: number | null;
  currentTaskDesc?: string | null;
  error?: { detail?: string; status_code?: number } | null;
  result?: { clawExitCode?: number; outputText?: string } | null;
};

let seq = 0;
function nextId(prefix: string): string {
  seq += 1;
  return `${prefix}-${seq}`;
}

function resetSeq(): void {
  seq = 0;
}

function push(
  out: TimelineEntry[],
  tsMs: number,
  tags: string[],
  message: string,
  isError: boolean,
  prefix: string,
): void {
  out.push({
    id: nextId(prefix),
    tsMs,
    tags,
    message,
    isError,
  });
}

function traceTs(row: TraceRow): number | null {
  const t = row.timestamp_ms ?? row.timestampMs;
  if (typeof t === "number" && t > 0) return t;
  return null;
}

function traceMessage(row: TraceRow): string {
  const ty = String(row.type ?? "");
  if (ty === "session_trace") {
    const name = String(row.name ?? "event");
    const attrs = row.attributes as Record<string, unknown> | undefined;
    const err =
      attrs?.error_preview ?? attrs?.error ?? attrs?.status ?? attrs?.tool_name;
    if (err != null && String(err).trim()) {
      return `${name} · ${String(err).slice(0, 120)}`;
    }
    return name;
  }
  if (ty === "agent_trace") {
    const kind = String(row.kind ?? "agent");
    return `agent.${kind}`;
  }
  return ty || "trace";
}

function tapMessage(ev: TapEventRow): string {
  const ty = String(ev.type ?? "event");
  if (ty === "text.delta" && typeof ev.text === "string") {
    const t = ev.text.trim();
    return t.length > 80 ? `text.delta · ${t.slice(0, 77)}…` : `text.delta · ${t}`;
  }
  if (ty === "solve.failed" && typeof ev.detail === "string") {
    return `solve.failed · ${ev.detail}`;
  }
  if (ty === "solve.finished") {
    const st = ev.status != null ? String(ev.status) : "done";
    return `solve.finished · ${st}`;
  }
  return ty;
}

function isErrorTap(ty: string): boolean {
  return ty === "solve.failed" || ty === "interrupt.required";
}

function isErrorTrace(row: TraceRow): boolean {
  const name = String(row.name ?? "");
  return (
    name.includes("failed") ||
    name.includes("error") ||
    name === "turn_failed" ||
    name === "llm_request_error"
  );
}

export function buildSessionTimeline(input: {
  task: TaskSnapshot;
  progressHistory: TaskProgressRow[];
  tapEvents: TapEventRow[];
  traceTail: TraceRow[];
}): TimelineEntry[] {
  resetSeq();
  const out: TimelineEntry[] = [];
  const { task, progressHistory, tapEvents, traceTail } = input;

  const baseMs = task.createdAtMs > 0 ? task.createdAtMs : Date.now();

  if (task.createdAtMs > 0) {
    push(out, task.createdAtMs, ["gw", "task"], `task · ${task.status || "created"}`, false, "task");
  }
  if (task.startedAtMs && task.startedAtMs > 0) {
    push(out, task.startedAtMs, ["gw", "task"], "task · running", false, "run");
  }

  let lastProgress = baseMs;
  for (const p of progressHistory) {
    if (!p.updatedAtMs) continue;
    const msg = p.currentTaskDesc?.trim() || p.phase || "progress";
    const prev = out[out.length - 1];
    if (prev?.message === msg && prev.tags.includes("progress")) continue;
    push(
      out,
      p.updatedAtMs,
      ["gw", "progress"],
      msg,
      false,
      "prog",
    );
    lastProgress = p.updatedAtMs;
  }

  tapEvents.forEach((ev, i) => {
    const ty = String(ev.type ?? "");
    const ts = baseMs + i + 1;
    const anchor = Math.max(lastProgress, task.startedAtMs ?? baseMs) + i + 1;
    push(
      out,
      Math.max(ts, anchor),
      ["gw", "tap"],
      tapMessage(ev),
      isErrorTap(ty),
      "tap",
    );
  });

  for (const row of traceTail) {
    const ts = traceTs(row);
    if (ts == null) continue;
    push(
      out,
      ts,
      ["trace"],
      traceMessage(row),
      isErrorTrace(row),
      "tr",
    );
  }

  if (task.finishedAtMs && task.finishedAtMs > 0) {
    const failed = task.status === "failed";
    let msg = `task · ${task.status}`;
    if (failed && task.error?.detail) {
      msg = `task · failed · ${task.error.detail}`;
    }
    push(out, task.finishedAtMs, ["gw", "task"], msg, failed, "done");
  }

  out.sort((a, b) => a.tsMs - b.tsMs || a.id.localeCompare(b.id));
  return out;
}

export function formatTimelineTime(tsMs: number): string {
  if (!tsMs || tsMs <= 0) return "—";
  const d = new Date(tsMs);
  return d.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

export function errorStripFromTask(task: TaskSnapshot): string | null {
  if (task.status !== "failed") return null;
  const parts: string[] = [];
  const code = task.result?.clawExitCode;
  if (code != null && code !== 0) {
    parts.push(`clawExitCode=${code}`);
  }
  if (task.error?.detail) {
    parts.push(task.error.detail);
  } else if (task.result?.outputText?.trim()) {
    parts.push(task.result.outputText.trim().slice(0, 240));
  }
  return parts.length ? parts.join(" · ") : null;
}
