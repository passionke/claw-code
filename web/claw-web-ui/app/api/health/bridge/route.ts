import { NextResponse } from "next/server";

/** BFF health proxy (avoids browser CORS to :8090). Author: kejiqing */
export async function GET() {
  const base = (process.env.CLAW_AGUI_BRIDGE_URL ?? "http://127.0.0.1:8090").replace(
    /\/$/,
    "",
  );
  try {
    const res = await fetch(`${base}/healthz`, { cache: "no-store" });
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
