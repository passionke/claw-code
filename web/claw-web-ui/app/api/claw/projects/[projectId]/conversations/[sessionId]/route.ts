import { NextRequest, NextResponse } from "next/server";
import { getSessionRecord, saveSessionMessages } from "@/lib/claw-conversation-db";
import type { ClawTunnelMessage } from "@/lib/claw-conversation-types";
import { resolveUserAndProject } from "@/lib/claw-project-auth";
import { pgConfigured } from "@/lib/claw-pg";

type Ctx = { params: Promise<{ projectId: string; sessionId: string }> };

function noPg() {
  return NextResponse.json({ error: "CLAW_WEB_DATABASE_URL not set" }, { status: 503 });
}

export async function GET(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) return noPg();
  const { projectId, sessionId } = await ctx.params;
  try {
    const { user } = await resolveUserAndProject(req, projectId);
    const record = await getSessionRecord(user.userId, projectId, sessionId);
    if (!record) {
      return NextResponse.json({ error: "session not found" }, { status: 404 });
    }
    return NextResponse.json(record);
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}

export async function PUT(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) return noPg();
  const { projectId, sessionId } = await ctx.params;
  try {
    const { user } = await resolveUserAndProject(req, projectId);
    const body = (await req.json()) as { messages: ClawTunnelMessage[] };
    const record = await saveSessionMessages(
      user.userId,
      projectId,
      sessionId,
      body.messages ?? [],
    );
    return NextResponse.json(record);
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}
