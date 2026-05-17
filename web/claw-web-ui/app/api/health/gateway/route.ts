import { NextResponse } from "next/server";
import { gatewayBaseUrl } from "@/lib/claw-config";

/** BFF health proxy (avoids browser CORS to :8088). Author: kejiqing */
export async function GET() {
  const base = gatewayBaseUrl();
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
