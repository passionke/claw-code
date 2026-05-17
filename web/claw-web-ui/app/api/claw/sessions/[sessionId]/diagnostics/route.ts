import { NextRequest, NextResponse } from "next/server";
import {
  buildSessionTimeline,
  errorStripFromTask,
  type TapEventRow,
  type TaskProgressRow,
  type TaskSnapshot,
  type TraceRow,
} from "@/lib/build-session-timeline";
import { defaultDsId, gatewayBaseUrl } from "@/lib/claw-config";

function dsIdFromRequest(req: NextRequest): number {
  const q = req.nextUrl.searchParams.get("dsId");
  if (q) {
    const n = Number.parseInt(q, 10);
    if (Number.isFinite(n) && n > 0) return n;
  }
  const header = req.headers.get("x-claw-ds-id");
  if (header) {
    const n = Number.parseInt(header, 10);
    if (Number.isFinite(n) && n > 0) return n;
  }
  const cookie = req.cookies.get("claw_ds_id")?.value;
  if (cookie) {
    const n = Number.parseInt(cookie, 10);
    if (Number.isFinite(n) && n > 0) return n;
  }
  return defaultDsId();
}

function parseNdjson(text: string): TapEventRow[] {
  const rows: TapEventRow[] = [];
  for (const line of text.split("\n")) {
    const t = line.trim();
    if (!t) continue;
    try {
      rows.push(JSON.parse(t) as TapEventRow);
    } catch {
      /* skip bad line */
    }
  }
  return rows;
}

/** BFF: execution + events + task → correlated timeline. Author: kejiqing */
export async function GET(
  req: NextRequest,
  ctx: { params: Promise<{ sessionId: string }> },
) {
  const { sessionId } = await ctx.params;
  const dsId = dsIdFromRequest(req);
  const base = gatewayBaseUrl();
  const headers = { "x-claw-ds-id": String(dsId) };

  try {
    const execUrl = `${base}/v1/sessions/${encodeURIComponent(sessionId)}/execution?ds_id=${dsId}&include_trace=true`;
    const eventsUrl = `${base}/v1/events/${encodeURIComponent(sessionId)}`;
    const taskUrl = `${base}/v1/tasks/${encodeURIComponent(sessionId)}?dsId=${dsId}`;

    const [execRes, eventsRes, taskRes] = await Promise.all([
      fetch(execUrl, { cache: "no-store", headers }),
      fetch(eventsUrl, { cache: "no-store", headers }),
      fetch(taskUrl, { cache: "no-store", headers }),
    ]);

    if (execRes.status === 404) {
      let gatewayDetail: string | undefined;
      try {
        const body = (await execRes.json()) as { detail?: string };
        gatewayDetail = body.detail;
      } catch {
        /* ignore */
      }
      return NextResponse.json(
        {
          error: "session not found",
          detail:
            gatewayDetail ??
            `sessionId 未在 gateway 索引中（dsId=${dsId}）。多为 mock 示例 ID、未发过消息，或 gateway 重建后会话表已清空。`,
          sessionId,
          dsId,
        },
        { status: 404 },
      );
    }

    const execution = execRes.ok ? await execRes.json() : null;
    const eventsText = eventsRes.ok ? await eventsRes.text() : "";
    const taskBody = taskRes.ok ? await taskRes.json() : null;

    const execTask = execution?.task as Record<string, unknown> | undefined;
    const task: TaskSnapshot = {
      status: String(taskBody?.status ?? execTask?.status ?? "unknown"),
      createdAtMs: Number(taskBody?.createdAtMs ?? execTask?.createdAtMs ?? 0),
      startedAtMs: (taskBody?.startedAtMs ?? execTask?.startedAtMs) as number | null,
      finishedAtMs: (taskBody?.finishedAtMs ?? execTask?.finishedAtMs) as number | null,
      currentTaskDesc: (taskBody?.currentTaskDesc ?? execTask?.currentTaskDesc) as
        | string
        | null,
      error: taskBody?.error as TaskSnapshot["error"],
      result: taskBody?.result as TaskSnapshot["result"],
    };

    const progressHistory = (execution?.progressHistory ?? []) as TaskProgressRow[];
    const traceTail = (execution?.traceTail ?? []) as TraceRow[];
    const tapEvents = parseNdjson(eventsText);
    const timeline = buildSessionTimeline({
      task,
      progressHistory,
      tapEvents,
      traceTail,
    });

    return NextResponse.json({
      sessionId,
      dsId,
      taskStatus: task.status,
      errorStrip: errorStripFromTask(task),
      sessionHomeRel: execution?.sessionHomeRel ?? null,
      timeline,
    });
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e), sessionId, dsId },
      { status: 502 },
    );
  }
}
