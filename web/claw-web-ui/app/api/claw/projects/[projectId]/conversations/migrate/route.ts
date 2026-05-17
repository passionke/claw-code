import { NextRequest, NextResponse } from "next/server";
import { migrateProject } from "@/lib/claw-conversation-db";
import type { ClawSessionRecord } from "@/lib/claw-conversation-types";
import { resolveUserAndProject } from "@/lib/claw-project-auth";
import { pgConfigured } from "@/lib/claw-pg";

type Ctx = { params: Promise<{ projectId: string }> };

export async function POST(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) {
    return NextResponse.json({ error: "CLAW_WEB_DATABASE_URL not set" }, { status: 503 });
  }
  const { projectId } = await ctx.params;
  try {
    const { user } = await resolveUserAndProject(req, projectId);
    const body = (await req.json()) as {
      activeSessionId: string | null;
      sessions: ClawSessionRecord[];
    };
    await migrateProject(
      user.userId,
      projectId,
      body.activeSessionId ?? null,
      body.sessions ?? [],
    );
    return NextResponse.json({ ok: true });
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}
