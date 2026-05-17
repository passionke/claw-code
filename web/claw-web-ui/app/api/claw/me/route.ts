import { NextRequest, NextResponse } from "next/server";
import { COOKIE_USER_ID, resolveUserFromRequest } from "@/lib/claw-web-auth";
import { pgConfigured } from "@/lib/claw-pg";

/** GET current user (creates dev-local row if needed). Author: kejiqing */
export async function GET(req: NextRequest) {
  if (!pgConfigured()) {
    return NextResponse.json(
      { error: "CLAW_WEB_DATABASE_URL not set" },
      { status: 503 },
    );
  }
  try {
    const user = await resolveUserFromRequest(req);
    const res = NextResponse.json(user);
    res.cookies.set(COOKIE_USER_ID, user.userId, {
      path: "/",
      sameSite: "lax",
      httpOnly: true,
      maxAge: 60 * 60 * 24 * 365,
    });
    return res;
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}
