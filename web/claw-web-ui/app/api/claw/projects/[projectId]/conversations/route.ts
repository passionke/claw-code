import { randomUUID } from "node:crypto";
import { NextRequest, NextResponse } from "next/server";
import { createSessionRecord, listSessionSummaries } from "@/lib/claw-conversation-db";
import { resolveUserAndProject } from "@/lib/claw-project-auth";
import { pgConfigured } from "@/lib/claw-pg";

type Ctx = { params: Promise<{ projectId: string }> };

function noPg() {
  return NextResponse.json(
    { error: "CLAW_WEB_DATABASE_URL not set; start claw-pg (./deploy/stack/gateway.sh pg-up)" },
    { status: 503 },
  );
}

/** GET list / POST create session. Author: kejiqing */
export async function GET(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) return noPg();
  const { projectId } = await ctx.params;
  try {
    const { user } = await resolveUserAndProject(req, projectId);
    const data = await listSessionSummaries(user.userId, projectId);
    return NextResponse.json({ ...data, userId: user.userId });
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}

export async function POST(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) return noPg();
  const { projectId } = await ctx.params;
  try {
    const { user } = await resolveUserAndProject(req, projectId);
    const body = (await req.json()) as { sessionId?: string };
    const sessionId = body.sessionId?.trim() || randomUUID();
    const record = await createSessionRecord(user.userId, projectId, sessionId, []);
    return NextResponse.json(record);
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}
