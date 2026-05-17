import { NextRequest, NextResponse } from "next/server";
import { defaultDsId } from "@/lib/claw-config";
import { ensureProjectForUser, listProjectsForUser } from "@/lib/claw-project-db";
import { resolveUserFromRequest } from "@/lib/claw-web-auth";
import { pgConfigured } from "@/lib/claw-pg";

function noPg() {
  return NextResponse.json({ error: "CLAW_WEB_DATABASE_URL not set" }, { status: 503 });
}

/** GET user's projects; POST ensure project for dsId. Author: kejiqing */
export async function GET(req: NextRequest) {
  if (!pgConfigured()) return noPg();
  try {
    const user = await resolveUserFromRequest(req);
    const projects = await listProjectsForUser(user.userId);
    return NextResponse.json({ projects });
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}

export async function POST(req: NextRequest) {
  if (!pgConfigured()) return noPg();
  try {
    const user = await resolveUserFromRequest(req);
    const body = (await req.json()) as { dsId?: number; title?: string };
    const dsId =
      body.dsId != null && Number.isFinite(body.dsId) && body.dsId >= 1
        ? body.dsId
        : defaultDsId();
    const project = await ensureProjectForUser(user.userId, dsId, { title: body.title });
    return NextResponse.json(project);
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}
