import { NextRequest, NextResponse } from "next/server";
import {
  archiveSessionRecord,
  deleteSessionRecord,
  getSessionRecord,
  saveSessionMessages,
} from "@/lib/claw-conversation-db";
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

export async function PATCH(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) return noPg();
  const { projectId, sessionId } = await ctx.params;
  try {
    const { user } = await resolveUserAndProject(req, projectId);
    const body = (await req.json()) as { archive?: boolean };
    if (body.archive) {
      await archiveSessionRecord(user.userId, projectId, sessionId);
      return NextResponse.json({ ok: true, sessionId, archived: true });
    }
    return NextResponse.json({ error: "unsupported patch" }, { status: 400 });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const status = msg.includes("not found") ? 404 : 500;
    return NextResponse.json({ error: msg }, { status });
  }
}

export async function DELETE(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) return noPg();
  const { projectId, sessionId } = await ctx.params;
  try {
    const { user } = await resolveUserAndProject(req, projectId);
    await deleteSessionRecord(user.userId, projectId, sessionId);
    return NextResponse.json({ ok: true, sessionId });
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const status = msg.includes("not found") ? 404 : 500;
    return NextResponse.json({ error: msg }, { status });
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
