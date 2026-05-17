/** Resolve current Claw Web user (dev cookie / future JWT). Author: kejiqing */

import type { NextRequest } from "next/server";
import { ensureUser } from "@/lib/claw-user-db";
import type { ClawWebUser } from "@/lib/claw-user-types";

export const COOKIE_USER_ID = "claw_user_id";

export function devUserId(): string {
  const id = process.env.CLAW_WEB_DEV_USER_ID?.trim();
  return id && id.length > 0 ? id : "dev-local";
}

export function devTenantId(): string | null {
  const t = process.env.CLAW_WEB_DEV_TENANT_ID?.trim();
  return t && t.length > 0 ? t : null;
}

function userIdFromBearer(req: NextRequest): string | null {
  const auth = req.headers.get("authorization");
  if (!auth?.startsWith("Bearer ")) return null;
  const token = auth.slice(7).trim();
  if (!token || token === "dev") return null;
  // Phase E: verify RS256 JWT; for now accept opaque dev token = user id
  if (token.length >= 8 && token.length <= 128 && /^[\w.-]+$/.test(token)) {
    return token;
  }
  return null;
}

/** BFF: ensure PG user row for this request. Author: kejiqing */
export async function resolveUserFromRequest(req: NextRequest): Promise<ClawWebUser> {
  const fromBearer = userIdFromBearer(req);
  const fromCookie = req.cookies.get(COOKIE_USER_ID)?.value?.trim();
  const userId = fromBearer || fromCookie || devUserId();
  return ensureUser({
    userId,
    tenantId: devTenantId(),
    displayName: userId === "dev-local" ? "Local Dev" : userId,
  });
}
