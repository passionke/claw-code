import { NextRequest, NextResponse } from "next/server";
import { setActiveSession } from "@/lib/claw-conversation-db";
import { resolveUserAndProject } from "@/lib/claw-project-auth";
import { pgConfigured } from "@/lib/claw-pg";

type Ctx = { params: Promise<{ projectId: string }> };

export async function PATCH(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) {
    return NextResponse.json({ error: "CLAW_WEB_DATABASE_URL not set" }, { status: 503 });
  }
  const { projectId } = await ctx.params;
  try {
    const { user } = await resolveUserAndProject(req, projectId);
    const body = (await req.json()) as { sessionId: string };
    if (!body.sessionId?.trim()) {
      return NextResponse.json({ error: "sessionId required" }, { status: 400 });
    }
    await setActiveSession(user.userId, projectId, body.sessionId.trim());
    return NextResponse.json({ ok: true, sessionId: body.sessionId.trim() });
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}
