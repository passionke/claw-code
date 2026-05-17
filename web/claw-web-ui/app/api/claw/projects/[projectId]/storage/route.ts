import { NextRequest, NextResponse } from "next/server";
import { putOssObject } from "@/lib/claw-oss-client";
import { ossHealthCheck } from "@/lib/claw-oss-client";
import { resolveUserAndProject } from "@/lib/claw-project-auth";
import { resolveProjectStorage } from "@/lib/claw-project-storage";
import { pgConfigured } from "@/lib/claw-pg";

type Ctx = { params: Promise<{ projectId: string }> };

/** GET project storage (OSS uri / local path). POST write .claw-project.json marker to OSS. Author: kejiqing */
export async function GET(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) {
    return NextResponse.json({ error: "CLAW_WEB_DATABASE_URL not set" }, { status: 503 });
  }
  const { projectId } = await ctx.params;
  try {
    const { project } = await resolveUserAndProject(req, projectId);
    const storage = resolveProjectStorage(project);
    const ossHealth =
      storage.protocol === "oss" ? await ossHealthCheck() : { ok: true as const };
    return NextResponse.json({ projectId, storage, ossHealth });
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}

export async function POST(req: NextRequest, ctx: Ctx) {
  if (!pgConfigured()) {
    return NextResponse.json({ error: "CLAW_WEB_DATABASE_URL not set" }, { status: 503 });
  }
  const { projectId } = await ctx.params;
  try {
    const { project } = await resolveUserAndProject(req, projectId);
    const storage = resolveProjectStorage(project);
    if (storage.protocol !== "oss" || !storage.prefix) {
      return NextResponse.json(
        { error: "project storage is local; set CLAW_OSS_* to use OSS" },
        { status: 400 },
      );
    }
    const manifest = {
      projectId: project.projectId,
      dsId: project.dsId,
      tenantId: project.tenantId,
      updatedAtMs: Date.now(),
    };
    const key = `${storage.prefix}.claw-project.json`;
    await putOssObject(key, JSON.stringify(manifest, null, 2), "application/json");
    return NextResponse.json({ ok: true, key, uri: `oss://${storage.bucket}/${key}` });
  } catch (e) {
    return NextResponse.json(
      { error: e instanceof Error ? e.message : String(e) },
      { status: 500 },
    );
  }
}
