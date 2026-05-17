/** BFF helper: project access checks. Author: kejiqing */

import type { NextRequest } from "next/server";
import { assertProjectAccess } from "@/lib/claw-project-db";
import type { ClawWebProject } from "@/lib/claw-project-types";
import { resolveUserFromRequest } from "@/lib/claw-web-auth";
import type { ClawWebUser } from "@/lib/claw-user-types";

export async function resolveUserAndProject(
  req: NextRequest,
  projectId: string,
): Promise<{ user: ClawWebUser; project: ClawWebProject }> {
  const user = await resolveUserFromRequest(req);
  const project = await assertProjectAccess(user.userId, projectId);
  return { user, project };
}
