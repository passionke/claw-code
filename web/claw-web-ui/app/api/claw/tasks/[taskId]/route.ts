import { NextRequest, NextResponse } from "next/server";
import { defaultDsId, gatewayBaseUrl } from "@/lib/claw-config";

function dsIdFromRequest(req: NextRequest): number {
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

/** BFF: task status + currentTaskDesc for sidebar progress (Phase B partial). Author: kejiqing */
export async function GET(
  req: NextRequest,
  ctx: { params: Promise<{ taskId: string }> },
) {
  const { taskId } = await ctx.params;
  const dsId = dsIdFromRequest(req);
  const base = gatewayBaseUrl();
  const qs = req.nextUrl.searchParams.get("dsId") ?? String(dsId);
  try {
    const url = `${base}/v1/tasks/${encodeURIComponent(taskId)}?dsId=${encodeURIComponent(qs)}`;
    const res = await fetch(url, {
      cache: "no-store",
      headers: { "x-claw-ds-id": String(dsId) },
    });
    const body = await res.text();
    return new NextResponse(body, {
      status: res.status,
      headers: { "Content-Type": "application/json" },
    });
  } catch (e) {
    return NextResponse.json(
      { status: "error", message: e instanceof Error ? e.message : String(e) },
      { status: 502 },
    );
  }
}
